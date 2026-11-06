use super::*;
#[cfg(test)]
use crate::looks::spektrafilm::Selection;
use crate::looks::{
    LookAsset, LookOrigin,
    authoring::DEFAULT_GRID,
    cache::Cache,
    material::{self, Selection as MaterialSelection},
};
use crate::project::Project;
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
mod photochemical_tests;
#[cfg(test)]
mod resolver_tests;
#[cfg(test)]
mod switch_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod ui_tests;

#[derive(Clone)]
pub(super) enum Source {
    Photochemical(MaterialSelection),
    Imported(PathBuf),
    Loaded(Arc<LookAsset>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Lut,
    FilmAndPaper,
}
impl Source {
    fn slot(&self) -> Slot {
        match self {
            Self::Photochemical(_) => Slot::FilmAndPaper,
            _ => Slot::Lut,
        }
    }
}
impl Slot {
    fn asset(self, project: &Project) -> Option<&Arc<LookAsset>> {
        match self {
            Self::Lut => project.output.lut.as_ref(),
            Self::FilmAndPaper => project.output.film_and_paper.as_ref(),
        }
    }
    fn set(self, project: &mut Project, asset: Option<Arc<LookAsset>>) {
        match self {
            Self::Lut => project.output.lut = asset,
            Self::FilmAndPaper => project.output.film_and_paper = asset,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Lut => "LUT",
            Self::FilmAndPaper => "Film & paper",
        }
    }
}

#[derive(Default)]
pub(super) struct Session {
    pub services: Option<Services>,
    pub loaded: Vec<Arc<LookAsset>>,
}

pub(super) struct Operation {
    #[cfg(test)]
    pub activity: ActivityReporter,
    pub selection: Option<MaterialSelection>,
    task: Option<async_executor::Task<()>>,
}
impl Operation {
    fn slot(&self) -> Slot {
        if self.selection.is_some() {
            Slot::FilmAndPaper
        } else {
            Slot::Lut
        }
    }
}

struct CacheOwner {
    value: Option<Cache>,
    limits: wgpu::Limits,
    #[cfg(test)]
    _directory: tempfile::TempDir,
}
#[cfg(test)]
type PreparedObserver = Arc<dyn Fn(bool, crate::cancellation::Cancellation) + Send + Sync>;
#[derive(Clone)]
pub(super) struct Services {
    cache: Arc<CacheOwner>,
    #[cfg(test)]
    prepared: Option<PreparedObserver>,
}
impl Services {
    pub fn new(limits: wgpu::Limits) -> Self {
        #[cfg(test)]
        let directory = tempfile::tempdir().expect("LUT cache fixture directory");
        let value = {
            #[cfg(test)]
            {
                Some(Cache::at(directory.path().into()))
            }
            #[cfg(not(test))]
            {
                Cache::default_location()
            }
        };
        Self {
            cache: Arc::new(CacheOwner {
                value,
                limits,
                #[cfg(test)]
                _directory: directory,
            }),
            #[cfg(test)]
            prepared: None,
        }
    }
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn observe_prepared(&mut self, observer: PreparedObserver) {
        self.prepared = Some(observer);
    }
    pub async fn prepare(self, source: Source) -> Result<Option<Arc<LookAsset>>, String> {
        crate::background::run(move |cancelled| {
            let result = prepare_asset(
                source,
                &cancelled.flag(),
                &self.cache.limits,
                self.cache.value.as_ref(),
            );
            #[cfg(test)]
            if let Some(observe) = &self.prepared {
                observe(matches!(&result, Ok(Some(_))), cancelled);
            }
            result
        })
        .await
        .map_err(|_| "LUT preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }
}

pub(super) fn prepare_asset(
    source: Source,
    cancel: &AtomicBool,
    limits: &wgpu::Limits,
    cache: Option<&Cache>,
) -> Result<Option<Arc<LookAsset>>, Error> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let asset = match source {
        Source::Loaded(asset) => asset.as_ref().clone(),
        Source::Photochemical(selection) => {
            let cached =
                cache.and_then(
                    |cache| match cache.load_photochemical(selection, DEFAULT_GRID) {
                        Ok(asset) => asset,
                        Err(error) => {
                            log::warn!(error = error.to_string().as_str(); "lut_cache_ignored");
                            None
                        }
                    },
                );
            match cached {
                Some(asset) => asset,
                None => {
                    let result = selection.bake(DEFAULT_GRID, cancel);
                    let asset = match result {
                        Ok(asset) => asset,
                        Err(_) if cancel.load(Ordering::Relaxed) => return Ok(None),
                        Err(error) => return Err(error),
                    };
                    if let Some(cache) = cache
                        && let Err(error) = cache.store_photochemical(selection, &asset)
                    {
                        log::warn!(error = error.to_string().as_str(); "lut_cache_write_failed");
                    }
                    asset
                }
            }
        }
        Source::Imported(path) => {
            let file = File::open(&path).map_err(crate::looks::failure)?;
            let mut bytes = Vec::new();
            file.take(crate::looks::MAX_IMPORT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(crate::looks::failure)?;
            if bytes.len() > crate::looks::MAX_IMPORT_BYTES {
                return Err(crate::looks::failure("LUT file exceeds the import budget"));
            }
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let text = std::str::from_utf8(&bytes).map_err(crate::looks::failure)?;
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("Imported LUT");
            LookAsset::import_cube(text, name)?
        }
    };
    if cancel.load(Ordering::Relaxed) {
        Ok(None)
    } else {
        crate::gpu::lut3d::Lut::require_limits(limits, asset.lut())?;
        Ok(Some(Arc::new(asset)))
    }
}

impl Editor {
    fn material_selection(&self) -> Option<MaterialSelection> {
        self.document
            .look
            .as_ref()
            .and_then(|op| op.selection)
            .or_else(|| {
                self.document
                    .state
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .and_then(|asset| MaterialSelection::from_origin(asset.origin()))
            })
    }
    fn look_request_allowed(&self, slot: Slot) -> bool {
        !self.dialogs.is_open()
            && (!self.interaction_blocked()
                || self
                    .document
                    .look
                    .as_ref()
                    .is_some_and(|op| op.slot() == slot))
    }
    fn select_negative(&mut self, negative: Option<material::Negative>, cx: &UiContext<'_, Self>) {
        if !self.look_request_allowed(Slot::FilmAndPaper) {
            return;
        }
        let current = self.material_selection();
        let paper = current
            .map(MaterialSelection::paper)
            .or_else(|| negative.map(material::Negative::default_paper));
        if let Some(selection) = paper.and_then(|paper| MaterialSelection::new(negative, paper)) {
            self.select_look(Source::Photochemical(selection), cx);
        } else if let (Some(previous), Some(negative)) = (current, negative) {
            self.request_material_switch(
                previous,
                MaterialSelection::new(Some(negative), negative.default_paper())
                    .expect("negative's default paper is supported"),
                cx,
            );
        }
    }
    #[cfg(test)]
    fn photochemical_selection(&self) -> Option<Selection> {
        match self.material_selection() {
            Some(MaterialSelection::Colour(selection)) => Some(selection),
            _ => None,
        }
    }
    fn select_paper(&mut self, paper: Option<material::Paper>, cx: &UiContext<'_, Self>) {
        if !self.look_request_allowed(Slot::FilmAndPaper) {
            return;
        }
        let current = self.material_selection();
        let negative = current.and_then(MaterialSelection::negative);
        match paper {
            Some(paper) => {
                if let Some(selection) = MaterialSelection::new(negative, paper) {
                    self.select_look(Source::Photochemical(selection), cx);
                } else if let Some(previous) = current {
                    self.request_material_switch(
                        previous,
                        MaterialSelection::new(None, paper)
                            .expect("paper-only treatment is supported"),
                        cx,
                    );
                }
            }
            None if current.is_some() && negative.is_none() => self.clear_slot(Slot::FilmAndPaper),
            _ => {}
        }
    }

    pub(super) fn select_look(&mut self, source: Source, cx: &UiContext<'_, Self>) {
        let slot = source.slot();
        // One preparation owns the busy interaction; the other stage keeps its accepted asset.
        if !self.look_request_allowed(slot) {
            return;
        }
        self.prepare_look(source, cx);
    }

    fn prepare_look(&mut self, source: Source, cx: &UiContext<'_, Self>) {
        let slot = source.slot();
        self.retire_look_selection();
        let cancelled = crate::cancellation::Cancellation::default();
        let activity = self
            .activities
            .start(ActivityOptions::new("Preparing look…").cancellable(
                "Cancel look preparation",
                "Cancelling look preparation…",
                cancelled,
            ));
        let selection = match &source {
            Source::Photochemical(s) => Some(*s),
            _ => None,
        };
        self.document.look = Some(Operation {
            #[cfg(test)]
            activity: activity.reporter(),
            selection,
            task: None,
        });

        let services = self.look.services.clone();
        let task = cx.spawn(move |cx| async move {
            let work = async {
                match services {
                    Some(services) => services.prepare(source).await,
                    None => Err("LUT services unavailable".into()),
                }
            };
            let cancel = async {
                activity.cancelled().await;
                Ok(None)
            };
            let result = futures_lite::future::or(cancel, work).await;
            drop(activity);
            cx.update(|editor| {
                editor.retire_look_selection();
                match result {
                    Ok(Some(asset)) => {
                        let before = editor.history_snapshot();
                        slot.set(&mut editor.document.state.project, Some(asset));
                        editor.remember_imported_look();
                        editor.invalidate_original();
                        editor.record_edit(before, slot.label(), None);
                    }
                    Ok(None) => editor.status = "LUT preparation cancelled".into(),
                    Err(message) => editor.look_failed(message),
                }
            });
        });
        if let Some(task) = task {
            self.document.look.as_mut().unwrap().task = Some(task);
        } else {
            self.retire_look_selection();
            self.look_failed("Application stopped".into());
        }
    }

    pub(super) fn retire_look_selection(&mut self) {
        self.document.look = None;
    }
    fn clear_slot(&mut self, slot: Slot) {
        if self.dialogs.is_open() {
            return;
        }
        if self
            .document
            .look
            .as_ref()
            .is_some_and(|op| op.slot() == slot)
        {
            self.retire_look_selection();
        }
        if slot.asset(&self.document.state.project).is_some() {
            let before = self.history_snapshot();
            slot.set(&mut self.document.state.project, None);
            self.invalidate_original();
            self.record_edit(before, slot.label(), None);
        }
    }
    pub(super) fn clear_film_and_paper(&mut self) {
        self.clear_slot(Slot::FilmAndPaper);
    }

    pub(super) fn clear_look(&mut self) {
        self.clear_slot(Slot::Lut);
    }
    pub(super) fn remember_imported_look(&mut self) {
        let Some(asset) = self
            .document
            .project
            .output
            .lut
            .as_ref()
            .filter(|asset| matches!(asset.origin(), LookOrigin::Imported))
        else {
            return;
        };
        if let Some(existing) = self
            .look
            .loaded
            .iter_mut()
            .find(|entry| entry.lut().id() == asset.lut().id())
        {
            *existing = asset.clone();
        } else {
            self.look.loaded.push(asset.clone());
        }
    }
}

fn loaded_lut_labels(assets: &[Arc<LookAsset>]) -> Vec<String> {
    let mut used: HashSet<String> = ["None".into(), "Load LUT…".into()].into_iter().collect();
    assets
        .iter()
        .map(|asset| {
            let mut name = asset.name().to_owned();
            let mut suffix = 2;
            while !used.insert(name.clone()) {
                name = format!("{} ({suffix})", asset.name());
                suffix += 1;
            }
            name
        })
        .collect()
}

impl Editor {
    fn request_material_switch(
        &mut self,
        previous: MaterialSelection,
        selection: MaterialSelection,
        cx: &UiContext<'_, Self>,
    ) {
        let explanation = if let Some(negative) = selection.negative() {
            format!(
                "{} with {} is not supported. Switch the paper to {}?",
                negative.name(),
                previous.paper().name(),
                selection.paper().name()
            )
        } else {
            format!(
                "{} with {} is not supported. Set Negative to None to use this paper?",
                previous
                    .negative()
                    .expect("switch clears a selected negative")
                    .name(),
                selection.paper().name()
            )
        };
        let response = self.dialogs.show(move |context, id, escape| {
            presentation::DecisionDialog::new(
                "Switch Film & paper?",
                &explanation,
                "Switch materials",
            )
            .show(context, id, escape)
        });
        self.document.dialog_action = cx.spawn(move |cx| async move {
            if response.await == Ok(presentation::DialogChoice::Confirm) {
                cx.update(|editor| editor.prepare_look(Source::Photochemical(selection), &cx));
            }
        });
    }
    pub(super) fn look_rendering_error(&self) -> Option<Error> {
        let view = &self.canvas_frame.displayed.as_ref()?.view;
        view.image.look_failure(view.look_id()?)
    }

    fn lut_controls_allowed(&self, slot: Slot) -> bool {
        self.look_request_allowed(slot) && !self.tool_interacting()
    }

    pub(super) fn look_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.imported_lut_controls(ui, cx);
        self.film_and_paper_controls(ui, cx);
    }

    fn imported_lut_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let labels = loaded_lut_labels(&self.look.loaded);
        let accepted = self.document.project.output.lut.as_ref();
        let name = accepted.map_or_else(
            || "None".to_owned(),
            |asset| {
                self.look
                    .loaded
                    .iter()
                    .position(|entry| entry.lut().id() == asset.lut().id())
                    .map_or_else(|| asset.name().to_owned(), |i| labels[i].clone())
            },
        );
        let changed = accepted.is_some();
        let mut choice = None;
        let mut clear = false;
        let mut import = false;
        ui.add_enabled_ui(self.lut_controls_allowed(Slot::Lut), |ui| {
            let (_, action) = presentation::section(
                ui,
                "rendering-lut",
                "LUT",
                presentation::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    presentation::inspector_dropdown(ui, "creative-lut", "LUT", &name, |ui| {
                        if ui.selectable_label(!changed, "None").clicked() && changed {
                            clear = true;
                        }
                        for (asset, label) in self.look.loaded.iter().zip(&labels) {
                            let selected =
                                self.document.look.is_none()
                                    && self.document.project.output.lut.as_ref().is_some_and(
                                        |current| current.lut().id() == asset.lut().id(),
                                    );
                            if ui
                                .push_id(asset.lut().id(), |ui| {
                                    ui.selectable_label(selected, label)
                                })
                                .inner
                                .clicked()
                                && !selected
                            {
                                choice = Some(Source::Loaded(asset.clone()));
                            }
                        }
                        ui.separator();
                        ui.small("ACEScct / AP1 (D60) input and output");
                        ui.small("Applied before Film & paper");
                        if ui.selectable_label(false, "Load LUT…").clicked() {
                            import = true;
                            ui.close();
                        }
                    });
                },
            );
            clear |= action.reset;
        });
        if clear {
            self.clear_look();
        }
        if import
            && let Some(path) = crate::platform::menu::with_native_dialog(|| {
                rfd::FileDialog::new()
                    .set_title("Load ACEScct / AP1 LUT")
                    .add_filter("3D LUT", &["cube"])
                    .pick_file()
            })
        {
            choice = Some(Source::Imported(path));
        }
        if let Some(source) = choice {
            self.select_look(source, cx);
        }
    }

    fn film_and_paper_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let selection = self.material_selection();
        let negative = selection.and_then(MaterialSelection::negative);
        let paper = selection.map(MaterialSelection::paper);
        let current_definition = self
            .document
            .look
            .as_ref()
            .is_some_and(|op| op.selection.is_some())
            || selection.is_some_and(|s| {
                self.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .is_some_and(|asset| asset.origin() == &s.origin())
            });
        let name = self
            .document
            .project
            .output
            .film_and_paper
            .as_ref()
            .map_or("None", |asset| asset.name())
            .to_owned();
        let changed = self.document.project.output.film_and_paper.is_some();
        let mut negative_choice = None;
        let mut paper_choice = None;
        let mut clear = false;
        ui.add_enabled_ui(self.lut_controls_allowed(Slot::FilmAndPaper), |ui| {
            let (_, action) = presentation::section(
                ui,
                "film-and-paper",
                "Film & paper",
                presentation::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    presentation::labelled_inspector_dropdown(
                        ui,
                        "negative-stock",
                        "Negative",
                        negative.map_or("None", |stock| stock.name()),
                        |ui| {
                            if ui.selectable_label(negative.is_none(), "None").clicked()
                                && negative.is_some()
                            {
                                negative_choice = Some(None);
                            }
                            for stock in material::Negative::all() {
                                let selected = negative == Some(stock) && current_definition;
                                let response = ui.selectable_label(selected, stock.name());
                                let response = if paper.is_some_and(|p| !p.supports_negative(stock))
                                {
                                    response.on_hover_text(
                                        "Select this negative to review the required paper change.",
                                    )
                                } else {
                                    response
                                };
                                if response.clicked() && !selected {
                                    negative_choice = Some(Some(stock));
                                    ui.close();
                                }
                            }
                        },
                    );
                    presentation::labelled_inspector_dropdown(
                        ui,
                        "print-paper",
                        "Paper",
                        paper.map_or("None", |medium| medium.name()),
                        |ui| {
                            let none = ui
                                .add_enabled_ui(negative.is_none(), |ui| {
                                    ui.selectable_label(paper.is_none(), "None")
                                })
                                .inner;
                            if none
                                .on_disabled_hover_text("Choose no negative to bypass the paper.")
                                .clicked()
                                && paper.is_some()
                            {
                                paper_choice = Some(None);
                            }
                            for medium in material::Paper::all() {
                                let selected = paper == Some(medium) && current_definition;
                                let response = ui.selectable_label(selected, medium.name());
                                let response =
                                    if negative.is_some_and(|n| !medium.supports_negative(n)) {
                                        response.on_hover_text(
                                            "Select this paper to review setting Negative to None.",
                                        )
                                    } else {
                                        response
                                    };
                                if response.clicked() && !selected {
                                    paper_choice = Some(Some(medium));
                                    ui.close();
                                }
                            }
                        },
                    );
                },
            );
            clear = action.reset;
        });
        if clear {
            self.clear_film_and_paper();
        } else if let Some(negative) = negative_choice {
            self.select_negative(negative, cx);
        } else if let Some(paper) = paper_choice {
            self.select_paper(paper, cx);
        }
    }
}

impl Editor {
    pub(super) fn look_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Error,
            title: "Could not prepare look",
            explanation: "Your existing look and layers are unchanged. Review the details, then choose another look or LUT file.",
            details,
        }));
    }
    pub(super) fn look_render_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Error,
            title: "Could not render look",
            explanation: "The look could not be rendered. Your selection and photographic layers are unchanged. Review the details before trying again.",
            details,
        }));
    }
}
