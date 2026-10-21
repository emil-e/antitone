#[cfg(test)]
#[path = "editor_document_workflow_tests.rs"]
mod tests;

use super::{
    Adjustments, DVec3, DocumentRequest, Editor, FailureKind, OutputTarget, Preparation, Prepared,
    RollSelection, Task, TaskProgress, TaskRequest, document_services::PreparationServices, dust,
    history, source_workflow,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct State {
    task: Option<async_executor::Task<()>>,
}

#[derive(Default)]
struct Progress {
    latest: Mutex<Option<(usize, usize)>>,
    changed: event_listener::Event,
}

impl Progress {
    fn report(&self, completed: usize, total: usize) {
        *self.latest.lock().unwrap() = Some((completed, total));
        self.changed.notify(usize::MAX);
    }
}

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Editor {
    pub(super) fn start_document(
        &mut self,
        work: Preparation,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        self.cancel_measurement();
        self.document.history.finish_group();
        self.history_pending = match &work {
            Preparation::New | Preparation::Open(_) => Some(history::Pending::Reset),
            Preparation::Import(..) | Preparation::Correction(..) => Some(history::Pending::Edit {
                before: Box::new(self.document.project.clone()),
                label: if matches!(&work, Preparation::Import(..)) {
                    "Import photos"
                } else {
                    "Correction"
                },
                photo: None,
            }),
            Preparation::History(_, _, direction) => Some(history::Pending::Restore {
                direction: *direction,
                cleanup: self
                    .document
                    .dust
                    .cleanup
                    .then_some(self.document.dust.show_masks),
            }),
        };
        let id =
            self.begin_document_operation(matches!(&work, Preparation::New | Preparation::Open(_)));
        self.tasks.begin(match &work {
            Preparation::Import(..) => Task::Import(id),
            Preparation::Open(_) => Task::Open(id),
            Preparation::New => Task::New(id),
            Preparation::Correction(..) => Task::Correction(id),
            Preparation::History(..) => Task::History(id),
        });
        self.document_failure =
            matches!(&work, Preparation::Correction(..)).then_some(FailureKind::FlatCorrection);
        self.new_project_pending = matches!(work, Preparation::New);
        let source = if matches!(&work, Preparation::Import(_, _, Some(_))) {
            self.document
                .visit
                .ready
                .as_ref()
                .map(|ready| (self.id, ready.clone()))
        } else {
            None
        };
        let request = DocumentRequest {
            work,
            source,
            correction: self.correction.clone(),
        };
        let source = self.source_services.clone();
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let retire = Retire(cancelled.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let preparation = async {
                let source = source.ok_or_else(|| "Document services unavailable".to_owned())?;
                PreparationServices(source)
                    .prepare(request, cancelled, move |done, total| {
                        produced.report(done, total)
                    })
                    .await
            };
            let reports = async {
                loop {
                    let changed = progress.changed.listen();
                    let latest = progress.latest.lock().unwrap().take();
                    if let Some((done, total)) = latest {
                        cx.update(|editor| editor.document_progress(id, done, total));
                    }
                    changed.await;
                }
            };
            let result = futures_lite::future::or(preparation, reports)
                .await
                .map(Box::new);
            cx.update(|editor| {
                editor.document_prepared(id, result);
                editor.acquire_working(&cx);
            });
        }) else {
            self.document_prepared(id, Err("Application stopped".into()));
            return;
        };
        self.candidate_operation.task = Some(task);
    }

    pub(super) fn document_progress(&mut self, id: u64, completed: usize, total: usize) {
        if self.document_pending == Some(id) {
            self.tasks.report(
                TaskRequest::Document(id),
                TaskProgress::Counted { completed, total },
            );
        }
    }
}

impl Editor {
    pub(super) fn document_prepared(&mut self, id: u64, result: Result<Box<Prepared>, String>) {
        if self.document_pending != Some(id) {
            return;
        }
        self.candidate_operation = State::default();
        self.tasks.finish(TaskRequest::Document(id));
        let new_project = std::mem::take(&mut self.new_project_pending);
        let failure_kind = self.document_failure.take();
        let pending_history = self.history_pending.take();
        self.document_pending = None;
        self.busy = false;
        match result {
            Ok(mut prepared) => {
                let flat_context = self.flat_edit.take();
                let previous_cache = self.document.cache().clone();
                self.thumbnails
                    .bind_sources(prepared.document.cache().clone());
                prepared.document.accept_cache(&previous_cache);
                if !Arc::ptr_eq(&previous_cache, prepared.document.cache()) {
                    previous_cache.close();
                }
                if self.reset_repairs_pending {
                    self.document.dust = dust::State::default();
                }
                self.reset_repairs_pending = false;
                let replacement = matches!(pending_history, Some(history::Pending::Reset));
                let same_visit = !replacement
                    && self.current_source == prepared.selected
                    && self
                        .document
                        .same_visit(prepared.ready.as_ref(), prepared.reuse_ready);
                self.document.visit.source = source_workflow::State::default();
                self.correction = prepared.correction;
                if replacement {
                    self.document = prepared.document.into();
                } else {
                    self.document.install(prepared.document);
                }
                self.remember_current_look();
                self.document.visit.adjustments = Adjustments::default();
                self.document.visit.mask_edit = None;
                self.grades.clear();
                if new_project {
                    self.tab = 0;
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
                self.id = id;
                if !same_visit {
                    self.replace_photo_scope(self.id);
                }
                self.current_source = prepared.selected;
                if !prepared.reuse_ready && !prepared.preserve_view {
                    self.roll_selection = RollSelection::default();
                }
                self.roll_selection
                    .reconcile(&self.document.project.order, self.current_source);
                self.selected = prepared
                    .selected
                    .and_then(|id| {
                        self.document
                            .project
                            .order
                            .iter()
                            .position(|candidate| *candidate == id)
                    })
                    .unwrap_or(0);
                self.roll = self.document.project.roll;
                self.roll_orientation = self.document.project.orientation;
                self.dmin = self.document.project.dmin.map(DVec3::from_array);
                self.dmax = self.document.project.dmax.map(DVec3::from_array);
                self.floor = self.document.project.input_floor.unwrap_or_else(|| {
                    prepared
                        .ready
                        .as_ref()
                        .map_or(self.floor, |ready| ready.floor)
                });
                if prepared.selected.is_none() {
                    self.floor = self.document.project.input_floor.unwrap_or(0.);
                }
                self.photo = prepared
                    .selected
                    .and_then(|id| self.document.project.source(id).ok())
                    .map(|source| source.photo)
                    .unwrap_or_default();
                self.photo_orientation = prepared
                    .selected
                    .and_then(|id| self.document.project.source(id).ok())
                    .map(|source| source.orientation)
                    .unwrap_or_default();
                self.crop = prepared
                    .selected
                    .and_then(|id| self.document.project.source(id).ok())
                    .and_then(|source| source.crop);
                self.document.visit.crop_edit = None;
                self.last_layout = None;
                self.target = match self.document.project.output.tiff {
                    crate::project::TiffTarget::Srgb => OutputTarget::Srgb,
                    crate::project::TiffTarget::DisplayP3 => OutputTarget::DisplayP3,
                };
                self.document.visit.ready = prepared.ready;
                if !prepared.reuse_ready && !prepared.preserve_view {
                    self.viewport.fit();
                    self.source = self.calibration().is_none();
                }
                self.pick = None;
                self.document.pick_view = None;
                self.document.pick_return = None;
                self.region = None;
                self.pick_drag = None;
                self.return_notice = None;
                self.overwrite = None;
                self.invalidate_original();
                if let Some(context) = flat_context {
                    self.viewport = context.viewport;
                    self.source = context.source;
                }
                self.status = if self.document.project.flat_field.is_some()
                    || prepared.preserve_view
                {
                    "Project ready. Flat-field changes keep Dmin/Dmax RGB; re-pick calibration for this setup."
                } else {
                    "Project ready"
                }
                .into();
                self.history_prepared(pending_history);
                if let Some(kind) = failure_kind {
                    self.resolve_failure(kind);
                }
            }
            Err(error) => {
                self.history_failed(pending_history);
                self.resume_source_after_document();
                self.status = format!("Project: {error}");
                if self.document.visit.source.pending() {
                    self.return_notice = Some(self.status.clone());
                }
                self.finish_flat_edit();
                self.source_epoch
                    .store(self.id, std::sync::atomic::Ordering::Relaxed);
                if let Some(kind) = failure_kind {
                    self.report_failure(kind, error);
                }
            }
        }
    }
}
