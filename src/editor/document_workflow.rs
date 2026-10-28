#[cfg(test)]
mod tests;

use super::{
    AdjustmentLayerEditor, DocumentRequest, Editor, FailureKind, OutputTarget, Preparation,
    Prepared, RollSelection, Task, TaskProgress, TaskRequest,
    document_services::PreparationServices, dust, history, source_workflow,
};
use crate::processing::InputSettings;
use std::sync::{Arc, Mutex};

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

impl Editor {
    pub(super) fn start_document(
        &mut self,
        work: Preparation,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before changing the document".into();
            return;
        }
        self.cancel_spatial();
        self.cancel_measurement();
        self.document.history.finish_group();
        self.history_pending = match &work {
            Preparation::New | Preparation::Open(_) => Some(history::Pending::Reset),
            Preparation::Import(..) | Preparation::Correction(..) | Preparation::Input(..) => {
                Some(history::Pending::Edit {
                    before: Box::new(self.document.state.project.clone()),
                    label: if matches!(&work, Preparation::Import(..)) {
                        "Import photos"
                    } else if matches!(&work, Preparation::Input(..)) {
                        "Input stage"
                    } else {
                        "Correction"
                    },
                    photo: None,
                })
            }
            Preparation::History(_, _, direction) => Some(history::Pending::Restore {
                direction: *direction,
                cleanup: self
                    .document
                    .visit
                    .dust
                    .cleanup
                    .then_some(self.document.visit.dust.show_masks),
            }),
        };
        let id =
            self.begin_document_operation(matches!(&work, Preparation::New | Preparation::Open(_)));
        self.tasks.begin(match &work {
            Preparation::Import(..) => Task::Import(id),
            Preparation::Open(_) => Task::Open(id),
            Preparation::New => Task::New(id),
            Preparation::Correction(..) => Task::Correction(id),
            Preparation::Input(..) => Task::Input(id),
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
        let cache = (!matches!(work, Preparation::New | Preparation::Open(_)))
            .then(|| self.document.cache.clone());
        let request = DocumentRequest {
            exposures: cache.as_ref().map(|_| self.document.exposures.clone()),
            sources: self
                .source_services
                .as_ref()
                .map(|services| services.provider(cache)),
            work,
            source,
            correction: self.document.correction.clone(),
        };
        let source = self.source_services.clone();
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
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
            Ok(prepared) => {
                let input_changed = self.document.input.is_negative()
                    != prepared.document.project.input.is_negative();
                let flat_context = self.document.flat_edit.take();

                if self.reset_repairs_pending {
                    self.document.dust = dust::State::default();
                }
                self.reset_repairs_pending = false;
                let replacement = matches!(pending_history, Some(history::Pending::Reset));
                let same_visit = !replacement
                    && self.document.visit.current_source == prepared.selected
                    && self
                        .document
                        .same_visit(prepared.ready.as_ref(), prepared.reuse_ready);
                let layer_selection =
                    if !replacement && self.document.visit.current_source == prepared.selected {
                        self.document.visit.adjustment_layers.selection_only()
                    } else {
                        AdjustmentLayerEditor::default()
                    };
                self.document.visit.source = source_workflow::State::default();
                if replacement {
                    self.document = self.document.replacement(prepared.document);
                } else {
                    self.document.install(prepared.document);
                }
                self.document.correction = prepared.correction;
                if let Some(ready) = &prepared.ready {
                    self.document.cache_ready(ready);
                }
                self.remember_current_look();
                self.document.visit.adjustment_layers = layer_selection;
                self.document.visit.mask_edit = None;
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
                self.document.visit.current_source = prepared.selected;
                if !prepared.reuse_ready && !prepared.preserve_view {
                    self.document.roll_selection = RollSelection::default();
                }
                self.document.roll_selection.reconcile(
                    &self.document.state.project.order,
                    self.document.visit.current_source,
                );
                self.document.selected = prepared
                    .selected
                    .and_then(|id| {
                        self.document
                            .state
                            .project
                            .order
                            .iter()
                            .position(|candidate| *candidate == id)
                    })
                    .unwrap_or(0);
                self.document.roll = self.document.state.project.roll;
                self.document.roll_orientation = self.document.state.project.orientation;
                self.load_input(self.document.state.project.input);
                if let InputSettings::Negative { input_floor, .. } = &mut self.document.input
                    && *input_floor == 0.
                {
                    *input_floor = crate::processing::CAMERA_INPUT_FLOOR;
                    self.document.state.project.input = self.document.input;
                }
                self.document.visit.adjustment_layer_settings = prepared
                    .selected
                    .and_then(|id| self.document.state.project.source(id).ok())
                    .and_then(|source| source.adjustment_layers.as_ref())
                    .and_then(|layers| layers.layers.last())
                    .map(|record| record.adjustment.photo)
                    .unwrap_or_default();
                self.refresh_adjustment_layer_selection();
                self.document.visit.photo_orientation = prepared
                    .selected
                    .and_then(|id| self.document.state.project.source(id).ok())
                    .map(|source| source.orientation)
                    .unwrap_or_default();
                self.document.visit.crop = prepared
                    .selected
                    .and_then(|id| self.document.state.project.source(id).ok())
                    .and_then(|source| source.crop);
                self.document.visit.crop_edit = None;
                self.document.visit.last_layout = None;
                self.document.film_emulation = self.document.state.project.output.film_emulation;
                self.document.target = match self.document.state.project.output.tiff {
                    crate::project::TiffTarget::Srgb => OutputTarget::Srgb,
                    crate::project::TiffTarget::DisplayP3 => OutputTarget::DisplayP3,
                };
                self.document.visit.ready = prepared.ready;
                if !prepared.reuse_ready && !prepared.preserve_view {
                    self.document.visit.viewport.fit();
                    self.document.source = !self.document.input.ready();
                }
                if input_changed {
                    self.document.source = !self.document.input.ready();
                }
                self.document.picking = Default::default();
                self.document.visit.pick_drag = None;
                self.document.visit.return_notice = None;
                self.document.export_dialog = None;
                self.document.export_summary = None;
                self.invalidate_original();
                if let Some(context) = flat_context {
                    self.document.visit.viewport = context.viewport;
                    self.document.source = context.source;
                }
                self.status = if !input_changed && self.document.state.project.active_flat_field().is_some()
                {
                    "Project ready. Flat-field changes keep Dmin/Dmax RGB; re-pick calibration for this setup."
                } else {
                    "Project ready"
                }
                .into();
                log::info!(request = id; "document_ready");
                self.history_prepared(pending_history);
                if let Some(kind) = failure_kind {
                    self.resolve_failure(kind);
                }
            }
            Err(error) => {
                log::warn!(
                    request = id,
                    error = error.as_str();
                    "document_failed"
                );
                self.history_failed(pending_history);
                self.resume_source_after_document();
                self.status = error.to_string();
                if self.document.visit.source.pending() {
                    self.document.visit.return_notice = Some(self.status.clone());
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
