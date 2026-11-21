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

pub(super) mod film_and_paper;
pub(super) mod lut;

#[cfg(test)]
use lut::loaded_lut_labels;

#[cfg(test)]
mod photochemical_tests;
#[cfg(test)]
mod resolver_tests;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod switch_tests;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transition_tests;
#[cfg(test)]
mod ui_tests;
#[cfg(test)]
mod workflow_tests;

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
    fn operation_mut(self, document: &mut DocumentScope) -> &mut Option<Operation> {
        match self {
            Self::Lut => &mut document.lut.pending,
            Self::FilmAndPaper => &mut document.film_and_paper.pending,
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
    _task: async_executor::Task<()>,
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
    fn look_request_allowed(&self) -> bool {
        !self.workspace_input.is_blocked()
    }

    fn lut_controls_allowed(&self) -> bool {
        self.look_request_allowed()
    }

    pub(super) fn look_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.imported_lut_controls(ui, cx);
        self.film_and_paper_controls(ui, cx);
    }

    pub(super) fn select_look(&mut self, source: Source, cx: &UiContext<'_, Self>) {
        if self.look_request_allowed() {
            self.prepare_look(source, cx);
        }
    }

    fn prepare_look(&mut self, source: Source, cx: &UiContext<'_, Self>) {
        let slot = source.slot();
        *slot.operation_mut(&mut self.document) = None;
        let (cancel_label, cancel_status) = match slot {
            Slot::Lut => ("Cancel LUT preparation", "Cancelling LUT preparation…"),
            Slot::FilmAndPaper => (
                "Cancel Film & paper preparation",
                "Cancelling Film & paper preparation…",
            ),
        };
        let activity = self.activities.start(
            ActivityOptions::new(format!("Preparing {}…", slot.label())).cancellable(
                cancel_label,
                cancel_status,
                crate::cancellation::Cancellation::default(),
            ),
        );
        let selection = match &source {
            Source::Photochemical(selection) => Some(*selection),
            _ => None,
        };
        #[cfg(test)]
        let reporter = activity.reporter();
        let services = self.look.services.clone();
        let task = cx.spawn(move |cx| async move {
            let work = async {
                let asset = match services {
                    Some(services) => services.prepare(source).await?,
                    None => return Err("LUT services unavailable".into()),
                };
                Ok(asset)
            };
            let cancel = async {
                activity.cancelled().await;
                Ok(None)
            };
            let result = futures_lite::future::or(cancel, work).await;
            drop(activity);
            cx.update(|editor| {
                *slot.operation_mut(&mut editor.document) = None;
                match result {
                    Ok(Some(asset)) => {
                        let result = (|| -> Result<(), Error> {
                            let mut edit = editor.edit_project(slot.label())?;
                            edit.update(|project| slot.set(project, Some(asset)));
                            editor.commit_project(edit)
                        })();
                        if let Err(error) = result {
                            editor.look_failed(error.to_string());
                        } else {
                            editor.remember_imported_look();
                        }
                    }
                    Ok(None) => editor.status = format!("{} preparation cancelled", slot.label()),
                    Err(message) => editor.look_failed(message),
                }
            });
        });
        if let Some(task) = task {
            *slot.operation_mut(&mut self.document) = Some(Operation {
                #[cfg(test)]
                activity: reporter,
                selection,
                _task: task,
            });
        } else {
            self.look_failed("Application stopped".into());
        }
    }

    fn clear_slot(&mut self, slot: Slot) {
        if self.dialogs.is_open() {
            return;
        }
        *slot.operation_mut(&mut self.document) = None;
        if slot.asset(&self.document.state.project).is_some() {
            let result = (|| -> Result<(), Error> {
                let mut edit = self.edit_project(slot.label())?;
                edit.update(|project| slot.set(project, None));
                self.commit_project(edit)
            })();
            if let Err(error) = result {
                self.look_failed(error.to_string());
            }
        }
    }

    pub(super) fn clear_film_and_paper(&mut self) {
        self.clear_slot(Slot::FilmAndPaper);
    }
}

impl Editor {
    pub(super) fn look_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::ui::Severity::Error,
            title: "Could not prepare look",
            explanation: "Your existing look and layers are unchanged. Review the details, then choose another look or LUT file.",
            details,
        }));
    }
}
