use super::*;
use crate::{cancellation::Cancellation, export::Overwrite, image_recipe::ImageRecipe};
use std::{
    collections::HashSet,
    ops::ControlFlow,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod tests;

#[derive(Default)]

pub(super) struct State {
    pub operation: Option<Operation>,
    pub draft: Option<Draft>,
    #[cfg(test)]
    pub summary: Option<Summary>,
}

/// The action retains cancellation feedback until its continuation acknowledges.
pub(super) struct Operation {
    #[cfg(test)]
    pub activity: ActivityReporter,
    pub cancelled: Cancellation,
    task: Option<async_executor::Task<()>>,
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

pub(super) struct PhotoRequest {
    pub input: source::Context,
    pub path: PathBuf,
    pub format: OutputFormat,
    pub recipe: ImageRecipe,
    pub overwrite: Overwrite,
}

enum WriteFailure {
    Photo(crate::Error),
    Infrastructure(crate::Error),
}

impl WriteFailure {
    fn error(&self) -> &crate::Error {
        match self {
            Self::Photo(error) | Self::Infrastructure(error) => error,
        }
    }
}

impl From<crate::Error> for WriteFailure {
    fn from(error: crate::Error) -> Self {
        Self::Photo(error)
    }
}

enum PhotoResult {
    Finished(Outcome),
    Cancelled,
    Fatal { path: PathBuf, error: String },
}

#[derive(Clone)]
pub(super) struct Services {
    renderers: Arc<crate::renderer::RendererCache>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    sources: source::Services,
}

impl Services {
    pub fn new(
        renderers: Arc<crate::renderer::RendererCache>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
        sources: source::Services,
    ) -> Self {
        Self {
            renderers,
            device,
            queue,
            submissions,
            sources,
        }
    }

    pub async fn export_batch(
        self,
        request: Request,
        cancelled: Cancellation,
        mut progress: impl FnMut(usize, usize),
    ) -> Summary {
        let total = request.photos.len();
        let mut summary = Summary {
            total,
            ..Default::default()
        };
        progress(0, total);
        for photo in request.photos {
            if cancelled.is_cancelled() {
                summary.stopped = true;
                break;
            }
            let published = summary
                .outcomes
                .iter()
                .filter_map(|outcome| match outcome {
                    Outcome::Exported(path) => Some(path.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let prepared = self
                .prepare_photo(
                    photo,
                    request.format,
                    request.collision,
                    &request.protected,
                    &published,
                    &cancelled,
                )
                .await;
            let result = match prepared {
                // Borrow so this iteration retains the source through progress reporting.
                ControlFlow::Continue(ref photo) => {
                    self.export_photo(
                        photo,
                        request.collision,
                        &request.protected,
                        &published,
                        &cancelled,
                    )
                    .await
                }
                ControlFlow::Break(PhotoResult::Cancelled) => {
                    summary.stopped = true;
                    break;
                }
                ControlFlow::Break(result) => result,
            };
            match result {
                PhotoResult::Finished(outcome) => summary.outcomes.push(outcome),
                PhotoResult::Cancelled => summary.stopped = true,
                PhotoResult::Fatal { path, error } => {
                    summary.outcomes.push(Outcome::Failed { path, error });
                    summary.stopped = true;
                }
            }
            progress(summary.outcomes.len(), total);
            if summary.stopped {
                break;
            }
        }
        summary
    }

    async fn prepare_photo(
        &self,
        photo: Photo,
        format: OutputFormat,
        policy: Collision,
        protected: &[PathBuf],
        published: &[PathBuf],
        cancelled: &Cancellation,
    ) -> ControlFlow<PhotoResult, PhotoRequest> {
        // Skip/protection checks precede acquisition; writes recheck afterwards.
        let path = match resolve_destination(
            &photo.path,
            policy,
            protected,
            published,
            "Export planning worker panicked",
        )
        .await
        {
            Ok(Some(path)) => path,
            Ok(None) => {
                return ControlFlow::Break(PhotoResult::Finished(Outcome::Skipped(photo.path)));
            }
            Err(result) => return ControlFlow::Break(result),
        };
        let input = match photo.input {
            Input::Ready(context) => Ok(context),
            Input::Acquire(request) => {
                self.sources
                    .clone()
                    .acquire(*request, cancelled.clone())
                    .await
            }
        };
        let input = match input {
            Ok(input) => input,
            Err(_) if cancelled.is_cancelled() => {
                return ControlFlow::Break(PhotoResult::Cancelled);
            }
            Err(error) => {
                return ControlFlow::Break(PhotoResult::Finished(Outcome::Failed { path, error }));
            }
        };
        ControlFlow::Continue(PhotoRequest {
            input,
            path: photo.path,
            format,
            recipe: photo.recipe,
            overwrite: if policy == Collision::Replace {
                Overwrite::Confirmed
            } else {
                Overwrite::Refuse
            },
        })
    }

    async fn export_photo(
        &self,
        photo: &PhotoRequest,
        policy: Collision,
        protected: &[PathBuf],
        published: &[PathBuf],
        cancelled: &Cancellation,
    ) -> PhotoResult {
        loop {
            if cancelled.is_cancelled() {
                return PhotoResult::Cancelled;
            }
            let path = match resolve_destination(
                &photo.path,
                policy,
                protected,
                published,
                "Destination check worker panicked",
            )
            .await
            {
                Ok(Some(path)) => path,
                Ok(None) => return PhotoResult::Finished(Outcome::Skipped(photo.path.clone())),
                Err(result) => return result,
            };
            let result = self
                .clone()
                .write_one(
                    PhotoRequest {
                        input: photo.input.clone(),
                        path: path.clone(),
                        format: photo.format,
                        recipe: photo.recipe.clone(),
                        overwrite: photo.overwrite,
                    },
                    cancelled.flag(),
                )
                .await;
            match result {
                Ok(path) => return PhotoResult::Finished(Outcome::Exported(path)),
                Err(WriteFailure::Photo(crate::Error::Cancelled)) => return PhotoResult::Cancelled,
                Err(WriteFailure::Photo(crate::Error::DestinationExists))
                    if policy == Collision::KeepBoth =>
                {
                    // Re-plan against the filesystem before rendering another candidate.
                    continue;
                }
                Err(WriteFailure::Photo(crate::Error::DestinationExists))
                    if policy == Collision::Skip =>
                {
                    return PhotoResult::Finished(Outcome::Skipped(path));
                }
                Err(error) => {
                    let error_text = error.error().to_string();
                    return if matches!(error, WriteFailure::Infrastructure(_)) {
                        PhotoResult::Fatal {
                            path,
                            error: error_text,
                        }
                    } else {
                        PhotoResult::Finished(Outcome::Failed {
                            path,
                            error: error_text,
                        })
                    };
                }
            }
        }
    }

    #[cfg(test)]
    pub async fn write(
        self,
        request: PhotoRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, String> {
        self.write_one(request, cancelled)
            .await
            .map_err(|error| error.error().to_string())
    }

    async fn write_one(
        self,
        request: PhotoRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, WriteFailure> {
        crate::background::run(move |_| {
            if cancelled.load(Ordering::Relaxed) {
                return Err(WriteFailure::Photo(crate::Error::Cancelled));
            }
            let scan = request.input.scan()?;
            let renderer = self
                .renderers
                .get(&self.device, &self.queue)
                .map_err(WriteFailure::Infrastructure)?;
            let stopped = || cancelled.load(Ordering::Relaxed);
            crate::gpu_export::job::write(
                &renderer,
                &self.device,
                (&self.queue, &self.submissions),
                crate::gpu_export::job::Job {
                    scan: &scan,
                    field: request.input.correction.as_ref().map(|c| c.gpu.clone()),
                    recipe: &request.recipe,
                    format: match request.format {
                        OutputFormat::Tiff(target) => crate::gpu_export::Format::Tiff(target),
                        OutputFormat::Exr(precision) => crate::gpu_export::Format::Exr(precision),
                    },
                },
                (&request.path, request.overwrite),
                &mut crate::dust_repair::Execution {
                    cancelled: &stopped,
                },
                || Ok(()),
            )?;
            Ok::<_, WriteFailure>(request.path)
        })
        .await
        .map_err(|_| {
            WriteFailure::Infrastructure(crate::Error::Gpu("Export worker panicked".to_owned()))
        })?
    }
}

async fn resolve_destination(
    path: &Path,
    policy: Collision,
    protected: &[PathBuf],
    published: &[PathBuf],
    panic_message: &'static str,
) -> Result<Option<PathBuf>, PhotoResult> {
    let failed_path = path.to_owned();
    let candidate = path.to_owned();
    let protected = protected.to_vec();
    let published = published.to_vec();
    match crate::background::run(move |_| destination(&candidate, policy, &protected, &published))
        .await
    {
        Ok(Ok(path)) => Ok(path),
        Ok(Err(error)) => Err(PhotoResult::Finished(Outcome::Failed {
            path: failed_path,
            error: error.to_string(),
        })),
        Err(_) => Err(PhotoResult::Fatal {
            path: failed_path,
            error: panic_message.into(),
        }),
    }
}

impl Editor {
    pub(super) fn export_photos(&mut self, request: Request, cx: &UiContext<'_, Self>) {
        let cancelled = Cancellation::default();
        let activity =
            self.activities
                .start(ActivityOptions::new("Exporting…").modal().cancellable(
                    "Cancel export",
                    "Cancelling export…",
                    cancelled.clone(),
                ));
        self.document.export.operation = Some(Operation {
            #[cfg(test)]
            activity: activity.reporter(),
            cancelled: cancelled.clone(),
            task: None,
        });
        self.document.thumbnails.cancel();
        self.status = "Exporting…".into();
        #[cfg(test)]
        {
            self.document.export.summary = None;
        }
        let total = request.photos.len();
        log::info!(total; "export_started");

        let services = self.export_services.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let summary = match services {
                Some(services) => {
                    services
                        .export_batch(request, cancelled, |completed, total| {
                            activity.progress(completed, total);
                        })
                        .await
                }
                None => Summary {
                    total,
                    stopped: true,
                    outcomes: vec![Outcome::Failed {
                        path: PathBuf::new(),
                        error: "Export services unavailable".into(),
                    }],
                },
            };
            drop(activity);
            cx.update(|editor| {
                log::info!(
                    total = summary.total,
                    exported = summary.exported(),
                    failed = summary.failed(),
                    stopped = summary.stopped;
                    "export_finished"
                );

                for outcome in &summary.outcomes {
                    if let Outcome::Failed { error, .. } = outcome {
                        log::warn!(
                            error = error.as_str();
                            "export_photo_failed"
                        );
                    }
                }
                editor.document.export.operation = None;
                if editor.interaction_blocked() {
                    // A captured batch cannot finish a newer foreground operation.
                    if summary.failed() > 0 {
                        editor.present_export_summary(summary);
                    }
                    return;
                }
                if summary.total == 1 && summary.exported() == 1 {
                    if let Outcome::Exported(path) = &summary.outcomes[0] {
                        editor.status = format!("Exported {}", path.display());
                    }

                    editor.present_export_summary(summary);
                } else if summary.total == 1 && summary.failed() == 1 {
                    if let Outcome::Failed { error, .. } = &summary.outcomes[0] {
                        editor.status = error.clone();
                        editor.export_failed(error.clone());
                    }
                } else if summary.stopped && summary.outcomes.is_empty() {
                    editor.status = "Export cancelled".into();
                } else {
                    editor.status = summary.text();
                    editor.present_export_summary(summary);
                }
            });
        }) else {
            self.document.export.operation = None;
            self.status = "Export cancelled".into();
            return;
        };
        self.document.export.operation.as_mut().unwrap().task = Some(task);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Selected,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Collision {
    KeepBoth,
    Skip,
    Replace,
    #[cfg(test)]
    Refuse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Tiff,
    Exr,
}

impl Kind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff => "tif",
            Self::Exr => "exr",
        }
    }
}

pub(super) struct Draft {
    pub scope: Scope,
    pub folder: PathBuf,
    pub kind: Kind,
    pub target: OutputTarget,
    pub precision: ExrPrecision,
    pub suffix: String,
    pub collision: Collision,

    pub error: Option<String>,
}

impl Draft {
    pub fn new(target: OutputTarget, folder: PathBuf) -> Self {
        Self {
            scope: Scope::Selected,
            folder,
            kind: Kind::Tiff,
            target,
            precision: ExrPrecision::Half,
            suffix: String::new(),
            collision: Collision::KeepBoth,

            error: None,
        }
    }

    pub fn format(&self) -> OutputFormat {
        match self.kind {
            Kind::Tiff => OutputFormat::Tiff(self.target),
            Kind::Exr => OutputFormat::Exr(self.precision),
        }
    }
}

pub(super) enum Input {
    Ready(source::Context),
    Acquire(Box<source::Request>),
}

pub(super) struct Photo {
    pub input: Input,
    pub recipe: ImageRecipe,
    pub path: PathBuf,
}

pub(super) struct Request {
    pub photos: Vec<Photo>,
    pub format: OutputFormat,
    pub collision: Collision,
    pub protected: Vec<PathBuf>,
}

#[derive(Clone)]
pub(super) enum Outcome {
    Exported(PathBuf),
    Skipped(PathBuf),
    Failed { path: PathBuf, error: String },
}

#[derive(Clone, Default)]
pub(super) struct Summary {
    pub total: usize,
    pub outcomes: Vec<Outcome>,
    pub stopped: bool,
}

impl Summary {
    pub fn exported(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Exported(_)))
            .count()
    }
    pub fn skipped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Skipped(_)))
            .count()
    }
    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Failed { .. }))
            .count()
    }
    pub fn unattempted(&self) -> usize {
        self.total - self.outcomes.len()
    }
    pub fn text(&self) -> String {
        format!(
            "{} exported · {} skipped · {} failed · {} remaining",
            self.exported(),
            self.skipped(),
            self.failed(),
            self.unattempted()
        )
    }
}

pub(super) fn filename(source: &Path, suffix: &str, kind: Kind) -> Result<PathBuf, Error> {
    if suffix
        .chars()
        .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
    {
        return Err(Error::InvalidParameter(
            "filename suffix contains a path separator or control character",
        ));
    }
    let mut name = source
        .file_stem()
        .filter(|s| !s.is_empty())
        .ok_or(Error::InvalidParameter("source has no filename"))?
        .to_os_string();
    name.push(suffix);
    name.push(".");
    name.push(kind.extension());
    Ok(PathBuf::from(name))
}

pub(super) fn numbered(path: &Path, index: usize) -> PathBuf {
    let mut stem = path.file_stem().unwrap_or_default().to_os_string();
    stem.push(format!(" ({index})"));
    if let Some(extension) = path.extension() {
        stem.push(".");
        stem.push(extension);
    }
    path.with_file_name(stem)
}

pub(super) fn destination(
    path: &Path,
    policy: Collision,
    protected: &[PathBuf],
    published: &[PathBuf],
) -> Result<Option<PathBuf>, Error> {
    let mut candidate = path.to_owned();
    let mut number = 2;
    loop {
        require_safe(&candidate, protected)?;
        // The destination filesystem decides name equivalence. In particular,
        // normalization/case aliases must not replace a previous batch output.
        if published
            .iter()
            .any(|path| same_destination(&candidate, path))
        {
            candidate = numbered(path, number);
            number += 1;
            continue;
        }
        let exists = match std::fs::symlink_metadata(&candidate) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(Error::Export(error.to_string())),
        };
        if !exists || policy == Collision::Replace {
            return Ok(Some(candidate));
        }
        if policy == Collision::Skip {
            return Ok(None);
        }
        if policy != Collision::KeepBoth {
            return Ok(Some(candidate));
        }
        candidate = numbered(path, number);
        number += 1;
    }
}

fn same_destination(a: &Path, b: &Path) -> bool {
    if document::same_path(a, b) {
        return true;
    }
    crate::sys::same_file(a, b)
}

pub(super) fn require_safe(path: &Path, protected: &[PathBuf]) -> Result<(), Error> {
    if protected.iter().any(|input| same_destination(path, input)) {
        return Err(Error::Export(
            "destination is a project document or source file".into(),
        ));
    }
    if path.is_dir() {
        return Err(Error::Export("destination is a directory".into()));
    }
    Ok(())
}

impl Editor {
    pub(super) fn export_ids(&self, scope: Scope) -> Vec<SourceId> {
        self.document
            .state
            .project
            .order
            .iter()
            .copied()
            .filter(|id| scope == Scope::All || self.document.roll_selection.contains(*id))
            .collect()
    }

    pub(super) fn capture_export(&mut self, draft: &Draft) -> Result<Request, Error> {
        if self.dust_search_pending() {
            return Err(Error::InvalidParameter(
                "wait for donor selection before exporting",
            ));
        }
        self.sync_document();
        self.validate_document_candidate(&self.document)?;
        if !draft.folder.is_absolute() || !draft.folder.is_dir() {
            return Err(Error::InvalidParameter(
                "choose an existing destination folder",
            ));
        }
        let ids = self.export_ids(draft.scope);
        if ids.is_empty() {
            return Err(Error::InvalidParameter("no photos selected for export"));
        }
        let mut protected = self
            .document
            .project
            .sources
            .iter()
            .map(|s| {
                self.document.source_path(s.id).and_then(|path| {
                    std::path::absolute(path).map_err(|error| Error::Export(error.to_string()))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(path) = &self.document.path {
            protected
                .push(std::path::absolute(path).map_err(|error| Error::Export(error.to_string()))?);
        }
        let mut names = HashSet::new();
        let mut photos = Vec::with_capacity(ids.len());
        for id in ids {
            let source_path = std::path::absolute(self.document.source_path(id)?)
                .map_err(|error| Error::Export(error.to_string()))?;
            let base = draft
                .folder
                .join(filename(&source_path, &draft.suffix, draft.kind)?);
            // Duplicate basenames must never replace an earlier file from this batch.
            let mut path = base.clone();
            let mut number = 2;
            while !names.insert(path.to_string_lossy().to_lowercase()) {
                path = numbered(&base, number);
                number += 1;
            }
            require_safe(&path, &protected)?;
            let expected = self.document.state.project.source(id)?;
            let input = if self.document.visit.current_source == Some(id) {
                self.source_context().map(Input::Ready)
            } else {
                None
            }
            .unwrap_or_else(|| {
                let expected = expected.clone();
                let wb = Some(expected.development.decode.fixed_wb);
                Input::Acquire(Box::new(source::Request {
                    reference: self.document.exposure_slot(&self.document.exposures, id),

                    input: self.input(),

                    file: self.document.file_key(&source_path),
                    sources: None,

                    path: source_path,
                    wb,
                    expected: Some(expected),
                    correction: self.document.correction.clone(),
                }))
            });
            photos.push(Photo {
                input,
                path,
                recipe: self.document.image_recipe(id)?,
            });
        }
        Ok(Request {
            photos,
            format: draft.format(),
            collision: draft.collision,
            protected,
        })
    }
}

#[derive(Default)]
struct Intent {
    close: bool,
    choose: bool,
    export: bool,
}

fn form(ui: &mut egui::Ui, draft: &mut Draft, selected: usize, all: usize) -> Intent {
    let mut intent = Intent::default();
    let form_inputs = (
        draft.scope,
        draft.folder.clone(),
        draft.kind,
        draft.target,
        draft.precision,
        draft.suffix.clone(),
        draft.collision,
    );
    ui.heading("Export photos");
    ui.add_space(8.);
    let height = (ui.ctx().content_rect().height() - 190.).max(80.);
    egui::ScrollArea::vertical()
        .max_height(height)
        .show(ui, |ui| {
            ui.strong("Photos");
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut draft.scope,
                    Scope::Selected,
                    format!("Selected photos ({selected})"),
                );
                ui.selectable_value(&mut draft.scope, Scope::All, format!("All photos ({all})"));
            });
            ui.add_space(8.);
            ui.strong("Destination");
            ui.horizontal(|ui| {
                let choose = presentation::TextAction::new("Choose…");
                let available =
                    (ui.available_width() - choose.width(ui) - ui.spacing().item_spacing.x).max(1.);
                let size = egui::vec2(available, presentation::CONTROL_HEIGHT);
                ui.allocate_ui_with_layout(
                    size,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_size(size);
                        ui.add(
                            egui::Label::new(if draft.folder.as_os_str().is_empty() {
                                "Choose a folder".into()
                            } else {
                                draft.folder.display().to_string()
                            })
                            .truncate(),
                        )
                        .on_hover_text(draft.folder.display().to_string());
                    },
                );
                intent.choose = choose.show(ui).clicked();
            });
            ui.add_space(8.);
            ui.strong("Output");
            presentation::labelled_row(ui, "Format", None, |ui, label| {
                egui::ComboBox::from_id_salt("export-format")
                    .width(240.)
                    .selected_text(match draft.kind {
                        Kind::Tiff => "TIFF",
                        Kind::Exr => "EXR",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.kind, Kind::Tiff, "TIFF");
                        ui.selectable_value(&mut draft.kind, Kind::Exr, "EXR");
                    })
                    .response
                    .labelled_by(label);
            });
            presentation::labelled_row(ui, "Target", None, |ui, label| {
                ui.add_enabled_ui(draft.kind == Kind::Tiff, |ui| {
                    egui::ComboBox::from_id_salt("export-target")
                        .width(240.)
                        .selected_text(if draft.kind == Kind::Exr {
                            "Scene-linear AP0"
                        } else {
                            match draft.target {
                                OutputTarget::Srgb => "sRGB",
                                OutputTarget::DisplayP3 => "Display P3",
                            }
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.target, OutputTarget::Srgb, "sRGB");
                            ui.selectable_value(
                                &mut draft.target,
                                OutputTarget::DisplayP3,
                                "Display P3",
                            );
                        })
                        .response
                        .labelled_by(label);
                });
            });
            if draft.kind == Kind::Exr {
                presentation::labelled_row(ui, "Precision", None, |ui, label| {
                    egui::ComboBox::from_id_salt("export-precision")
                        .width(240.)
                        .selected_text(match draft.precision {
                            ExrPrecision::Half => "Half float · 16-bit",
                            ExrPrecision::Full => "Full float · 32-bit",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.precision,
                                ExrPrecision::Half,
                                "Half float · 16-bit",
                            );
                            ui.selectable_value(
                                &mut draft.precision,
                                ExrPrecision::Full,
                                "Full float · 32-bit",
                            );
                        })
                        .response
                        .labelled_by(label);
                });
                ui.small("Creative look and display rendering are not applied.");
            } else {
                presentation::labelled_row(ui, "Bit depth", None, |ui, _| {
                    ui.label("16-bit RGB");
                });
                presentation::labelled_row(ui, "Compression", None, |ui, _| {
                    ui.label("Lossless · Deflate");
                });
            }
            ui.add_space(8.);
            ui.strong("Files");
            presentation::labelled_row(ui, "Filename", None, |ui, _| {
                ui.label("Original filename");
            });
            presentation::labelled_row(ui, "Suffix", None, |ui, label| {
                ui.add(egui::TextEdit::singleline(&mut draft.suffix).desired_width(240.))
                    .labelled_by(label);
            });
            if let Ok(name) = filename(
                std::path::Path::new("DSC01234.RAW"),
                &draft.suffix,
                draft.kind,
            ) {
                ui.small(format!("Example: {}", name.display()));
            }
            presentation::labelled_row(ui, "Existing files", None, |ui, label| {
                egui::ComboBox::from_id_salt("export-existing")
                    .width(240.)
                    .selected_text(match draft.collision {
                        Collision::KeepBoth => "Keep both",
                        Collision::Skip => "Skip",
                        Collision::Replace => "Replace…",
                        #[cfg(test)]
                        Collision::Refuse => "Refuse",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.collision, Collision::KeepBoth, "Keep both");
                        ui.selectable_value(&mut draft.collision, Collision::Skip, "Skip");
                        ui.selectable_value(&mut draft.collision, Collision::Replace, "Replace…");
                    })
                    .response
                    .labelled_by(label);
            });
            let inputs = (
                draft.scope,
                draft.folder.clone(),
                draft.kind,
                draft.target,
                draft.precision,
                draft.suffix.clone(),
                draft.collision,
            );
            if inputs != form_inputs {
                draft.error = None;
            }
            if let Some(error) = &draft.error {
                ui.label(error);
            }
        });
    ui.add_space(12.);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let count = if draft.scope == Scope::Selected {
            selected
        } else {
            all
        };
        let export = presentation::TextAction::new(&format!(
            "Export {count} {}",
            if count == 1 { "photo" } else { "photos" }
        ))
        .primary()
        .enabled(count > 0 && !draft.folder.as_os_str().is_empty())
        .show(ui);
        intent.export = export.clicked();
        let cancel = presentation::TextAction::new("Cancel").show(ui);
        intent.close = cancel.clicked();
        presentation::focus_default(ui, &cancel);
    });
    intent
}

impl Editor {
    pub(super) fn export_modal(&self) -> bool {
        self.document.export.draft.is_some()
    }

    pub(super) fn open_export_dialog(&mut self) {
        if self.tool_interacting() || self.document.visit.masks.editing() {
            self.status = "Finish the active interaction before exporting".into();
            return;
        }
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before exporting".into();
            return;
        }
        let folder = self.preferences.export_folder.clone().unwrap_or_default();
        self.document.export.draft = Some(Draft::new(self.document.target, folder));
    }

    pub(super) fn export_dialogs(
        &mut self,
        context: &egui::Context,
        escape: bool,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(mut draft) = self.document.export.draft.take() {
            let selected = self.export_ids(Scope::Selected).len();
            let all = self.export_ids(Scope::All).len();
            let response = presentation::modal(context, egui::Id::new("export-photos"), |ui| {
                ui.set_width((context.content_rect().width() - 80.).clamp(1., 500.));
                form(ui, &mut draft, selected, all)
            });
            if response.inner.close || presentation::escape_requested(&response, escape) {
                return;
            }
            if response.inner.choose {
                let mut picker = rfd::FileDialog::new().set_title("Export destination");
                if !draft.folder.as_os_str().is_empty() {
                    picker = picker.set_directory(&draft.folder);
                }
                if let Some(folder) =
                    crate::platform::menu::with_native_dialog(|| picker.pick_folder())
                {
                    draft.error = None;
                    draft.folder = folder;
                }
            }
            if response.inner.export {
                if draft.collision == Collision::Replace {
                    self.request_export_replacement(draft, cx);
                } else {
                    self.accept_export(draft, cx);
                }
            } else {
                self.document.export.draft = Some(draft);
            }
        }
    }

    fn present_export_summary(&mut self, summary: Summary) {
        #[cfg(test)]
        {
            self.document.export.summary = Some(summary.clone());
        }
        drop(self.dialogs.show(move |context, id, escape| {
            let response = presentation::modal(context, id, |ui| {
                ui.heading(if summary.stopped {
                    "Export stopped"
                } else {
                    "Export complete"
                });
                ui.label(summary.text());
                egui::ScrollArea::vertical()
                    .max_height(180.)
                    .show(ui, |ui| {
                        for outcome in &summary.outcomes {
                            match outcome {
                                Outcome::Failed { path, error } => {
                                    ui.label(format!("{}: {error}", path.display()));
                                }
                                Outcome::Skipped(path) => {
                                    ui.label(format!("Skipped {}", path.display()));
                                }
                                _ => {}
                            }
                        }
                    });
                let mut reveal = None;
                if let Some(Outcome::Exported(path)) = summary
                    .outcomes
                    .iter()
                    .find(|o| matches!(o, Outcome::Exported(_)))
                    && presentation::TextAction::new("Show in Finder")
                        .show(ui)
                        .clicked()
                {
                    reveal = Some(path.clone());
                }
                let ok = presentation::TextAction::new("OK").show(ui);
                presentation::focus_default(ui, &ok);
                (ok.clicked(), reveal)
            });
            if let Some(path) = &response.inner.1 {
                reveal_file(path.clone());
            }
            (response.inner.0 || presentation::escape_requested(&response, escape)).then_some(())
        }));
    }

    pub(super) fn request_export_replacement(&mut self, draft: Draft, cx: &UiContext<'_, Self>) {
        let detail = draft.folder.display().to_string();
        let response = self.dialogs.show(move |context, id, escape| {
            presentation::DecisionDialog::new("Replace existing exported files?",
                "Existing files with the generated names in the destination folder will be replaced. Completed replacements cannot be undone.",
                "Replace and export").warning().detail(&detail).show(context, id, escape)
        });
        self.document.dialog_action = cx.spawn(move |cx| async move {
            let Ok(choice) = response.await else { return };
            cx.update(|editor| {
                if choice == presentation::DialogChoice::Confirm {
                    editor.accept_export(draft, &cx);
                } else {
                    editor.document.export.draft = Some(draft);
                }
            });
        });
    }

    fn accept_export(&mut self, mut draft: Draft, cx: &UiContext<'_, Self>) {
        match self.capture_export(&draft) {
            Ok(request) => {
                self.preferences.export_folder = Some(draft.folder);
                self.persist_preferences();
                self.export_photos(request, cx);
            }
            Err(error) => {
                draft.error = Some(error.to_string());
                self.document.export.draft = Some(draft);
            }
        }
    }
}

fn reveal_file(path: PathBuf) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
    }
}

impl Editor {
    pub(super) fn export_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Error,
            title: "Could not export image",
            explanation: "Nothing was exported. The project and any existing destination file are unchanged. Review the details, then try exporting again.",
            details,
        }));
    }
}
