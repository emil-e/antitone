//! Captured photographic file output shared by application workflows.
use super::*;
use crate::{
    cancellation::Cancellation,
    export::{FileFormat, Overwrite, Plan},
    image_recipe::ImageRecipe,
};
use std::{
    collections::HashSet,
    ops::ControlFlow,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) struct PhotoRequest {
    pub input: source::Context,
    pub path: PathBuf,
    pub plan: Plan,
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
    evaluation: crate::evaluation::Context,
    looks: Arc<crate::photographic::stages::look::Looks>,
    pub(super) display: Arc<super::display::Warmup>,
    pub(super) queue: crate::gpu::submission::Queue,
    pub(super) sources: source::Services,
}

impl Services {
    pub fn new(
        evaluation: crate::evaluation::Context,
        looks: Arc<crate::photographic::stages::look::Looks>,
        display: Arc<super::display::Warmup>,
        queue: crate::gpu::submission::Queue,
        sources: source::Services,
    ) -> Self {
        Self {
            evaluation,
            looks,
            display,
            queue,
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
                    request.plan,
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
        plan: Plan,
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
        let mut input = match input {
            Ok(input) => input,
            Err(_) if cancelled.is_cancelled() => {
                return ControlFlow::Break(PhotoResult::Cancelled);
            }
            Err(error) => {
                return ControlFlow::Break(PhotoResult::Finished(Outcome::Failed { path, error }));
            }
        };
        input.correction = match futures_lite::future::or(
            async {
                cancelled.wait().await;
                Err("Export cancelled".to_owned())
            },
            photo.correction.acquire(),
        )
        .await
        {
            Ok(correction) => correction.into(),
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
            plan,
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
                        plan: photo.plan,
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
            let render = crate::gpu_export::job::Render::resolve(request.plan, || {
                let targets = self
                    .display
                    .targets()
                    .map_err(WriteFailure::Infrastructure)?;
                Ok::<_, WriteFailure>((self.looks.as_ref(), targets))
            })?;
            let stopped = || cancelled.load(Ordering::Relaxed);
            crate::gpu_export::job::write(
                &self.evaluation,
                &self.queue,
                crate::gpu_export::job::Job {
                    scan: &scan,
                    field: request.input.correction.gpu()?,
                    recipe: &request.recipe,
                    render,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Collision {
    KeepBoth,
    Skip,
    Replace,
    #[cfg(test)]
    Refuse,
}

pub(super) enum Input {
    Ready(source::Context),
    Acquire(Box<source::Request>),
}

pub(super) struct Photo {
    pub input: Input,
    pub correction: super::correction::Resources,
    pub recipe: ImageRecipe,
    pub path: PathBuf,
}

pub(super) struct Request {
    pub photos: Vec<Photo>,
    pub plan: Plan,
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

pub(super) fn filename(source: &Path, suffix: &str, format: FileFormat) -> Result<PathBuf, Error> {
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
    name.push(format.extension());
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

pub(super) enum Folders {
    One(PathBuf),
    PerPhoto(std::collections::HashMap<SourceId, PathBuf>),
}

pub(super) struct Options {
    pub folders: Folders,
    pub plan: Plan,
    pub suffix: String,
    pub collision: Collision,
}

impl Options {
    pub fn new(plan: Plan, folders: Folders) -> Self {
        Self {
            folders,
            plan,
            suffix: String::new(),
            collision: Collision::KeepBoth,
        }
    }
}

impl Editor {
    pub(super) fn capture_output(
        &self,
        options: &Options,
        ids: Vec<SourceId>,
    ) -> Result<Request, Error> {
        if self.editing() {
            return Err(Error::InvalidParameter(
                "finish or cancel the current edit before exporting",
            ));
        }

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
            let folder = match &options.folders {
                Folders::One(folder) => folder,
                Folders::PerPhoto(folders) => folders.get(&id).ok_or(Error::InvalidParameter(
                    "missing output folder for photograph",
                ))?,
            };
            if !folder.is_absolute() || !folder.is_dir() {
                return Err(Error::InvalidParameter(
                    "choose an existing destination folder",
                ));
            }
            let base = folder.join(filename(
                &source_path,
                &options.suffix,
                options.plan.format(),
            )?);
            // Duplicate basenames must never replace an earlier file from this batch.
            let mut path = base.clone();
            let mut number = 2;
            while !names.insert(path.to_string_lossy().to_lowercase()) {
                path = numbered(&base, number);
                number += 1;
            }
            require_safe(&path, &protected)?;
            let ready = (self.document.visit.current_source == Some(id))
                .then(|| self.source_context())
                .flatten();
            let input = match ready {
                Some(context) => Input::Ready(context),
                None => Input::Acquire(Box::new(source::Request {
                    source: self.document.source_input(id)?,
                    sources: None,
                    correction: self.correction(),
                })),
            };
            photos.push(Photo {
                correction: self.correction(),
                input,
                path,
                recipe: self.document.image_recipe(id)?,
            });
        }
        Ok(Request {
            photos,
            plan: options.plan,
            collision: options.collision,
            protected,
        })
    }
}
