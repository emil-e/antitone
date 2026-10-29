use super::{
    AdjustmentLayerEditor, Editor, Ready, SelectionTiming, Task, TaskRequest, UiContext, source,
};
use crate::processing::InputSettings;
use crate::source::SourceKey;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct State {
    task: Option<async_executor::Task<()>>,
    completion: Option<Result<Box<Ready>, String>>,
    pub key: Option<SourceKey>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.task.is_some() || self.completion.is_some()
    }

    pub fn matches(&self, ready: &Ready) -> bool {
        self.key
            .as_ref()
            .zip(
                SourceKey::from_revision(
                    &ready.path,
                    ready.file.clone(),
                    &ready.record.development,
                )
                .as_ref(),
            )
            .is_some_and(|(requested, prepared)| requested == prepared)
    }
}

impl Editor {
    pub(super) fn source_input(&self, path: PathBuf) -> source::Request {
        let expected = self
            .document
            .find_source(&path)
            .and_then(|id| self.document.state.project.source(id).ok())
            .cloned();
        let wb = self
            .document
            .state
            .project
            .acquisition_wb(expected.as_ref());
        let file = self.document.file_key(&path);
        source::Request {
            reference: expected.as_ref().map_or_else(Arc::default, |source| {
                self.document
                    .exposure_slot(&self.document.exposures, source.id)
            }),
            input: self.input(),
            path,
            wb,
            expected,
            correction: self.document.correction.clone(),
            file,
            sources: self.source_provider(),
        }
    }

    pub(super) fn load_source(
        &mut self,
        path: PathBuf,
        persistent: bool,
        cx: &UiContext<'_, Self>,
    ) {
        self.sync_document();
        if let Err(error) = self.validate_document_candidate(&self.document.state) {
            self.status = format!("Source not changed: {error}");
            return;
        }
        let started = std::time::Instant::now();
        let selecting_photo =
            persistent && self.active_pick().is_none() && self.document.flat_edit.is_none();
        self.document.visit.support = Default::default();
        self.seed_thumbnail();
        self.document.visit.pending_preview = None;
        self.document.visit.canvas.selection_timing = None;
        if self.document.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.document.thumbnails.cancel();
        self.document.visit.adjustment_layers = AdjustmentLayerEditor::default();
        self.document.visit.mask_edit = None;
        self.id = self.request_id();
        self.replace_photo_scope(self.id);
        self.document.visit.current_source = self.document.find_source(&path);
        if selecting_photo {
            self.document.roll_selection.reconcile(
                &self.document.state.project.order,
                self.document.visit.current_source,
            );
        }
        self.document.visit.adjustment_layer_settings = self
            .document
            .visit
            .current_source
            .and_then(|id| self.document.state.project.source(id).ok())
            .and_then(|source| source.adjustment_layers.as_ref())
            .and_then(|layers| layers.layers.last())
            .map(|record| record.adjustment.photo)
            .unwrap_or_default();
        self.refresh_adjustment_layer_selection();
        self.document.visit.photo_orientation = self
            .document
            .visit
            .current_source
            .and_then(|id| self.document.state.project.source(id).ok())
            .map(|source| source.orientation)
            .unwrap_or_default();
        self.document.visit.crop = self
            .document
            .visit
            .current_source
            .and_then(|id| self.document.state.project.source(id).ok())
            .and_then(|source| source.crop);
        self.document.visit.crop_edit = None;
        self.document.visit.canvas.last_layout = None;
        self.invalidate_original();
        self.document.visit.canvas.viewport.fit();
        self.document.export_dialog = None;
        self.document.export_summary = None;
        self.document.visit.return_notice = None;
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.document.visit.canvas.ready = None;
        self.document.visit.canvas.selection_timing =
            selecting_photo.then(|| Arc::new(SelectionTiming::new(self.id, started)));
        self.document.source = self
            .document
            .auto_crop_edit
            .as_ref()
            .map_or(!self.document.input.ready(), |d| d.source_view);
        self.busy = true;
        self.status = "Developing full-resolution RAW…".into();
        self.tasks.begin(Task::Photo(self.id));
        let request = self.source_input(path);
        self.document.visit.source.key = request.expected.as_ref().and_then(|source| {
            SourceKey::from_revision(&request.path, request.file.clone(), &source.development)
        });
        let view = self.document.visit.view;
        let services = self.source_services.clone();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => services.prepare(request, cancelled).await.map(Box::new),
                None => Err("Source services unavailable".into()),
            };
            cx.update(|editor| editor.source_result(view, result));
        }) else {
            self.source_result(view, Err("Application stopped".into()));
            return;
        };
        self.document.visit.source.task = Some(task);
        if selecting_photo
            && self.active_pick().is_none()
            && self.document.flat_edit.is_none()
            && let Some(source) = self.document.visit.current_source
        {
            let composition = self
                .document
                .state
                .project
                .source(source)
                .ok()
                .map(|source| self.canvas_composition(source.development.active_area.into(), None));
            if let Some(composition) = composition {
                match self.document.thumbnails.selected_preview(
                    self.id,
                    &self.document.state,
                    source,
                    composition,
                    self.document.correction.as_ref(),
                ) {
                    Ok(preview) => {
                        log::debug!(request = self.id, cache_hit = preview.is_some(), boundary = "admission", elapsed_ms = started.elapsed().as_secs_f64() * 1000.; "selected_switch");
                        self.document.visit.pending_preview = preview;
                    }
                    Err(error) => {
                        log::debug!(request = self.id, error = error.to_string().as_str(); "selected_preview_unavailable")
                    }
                }
            }
        }
    }

    pub(super) fn source_result(&mut self, view: u64, result: Result<Box<Ready>, String>) {
        if self.document.visit.view != view {
            return;
        }
        self.document.visit.source.task = None;
        self.document.visit.source.completion = Some(result);
        self.resume_source_completion();
    }

    fn resume_source_completion(&mut self) {
        if self.document_pending.is_some() {
            return;
        }
        let Some(result) = self.document.visit.source.completion.take() else {
            return;
        };
        let view = self.document.visit.view;
        self.document.visit.source = State::default();
        self.tasks.finish(TaskRequest::Photo(view));
        if result.is_err() {
            self.document.visit.pending_preview = None;
        }
        if let Some(timing) = &self.document.visit.canvas.selection_timing {
            log::debug!(
                request = view,
                success = result.is_ok(),
                boundary = "source_ready",
                elapsed_ms = timing.started.elapsed().as_secs_f64() * 1000.;
                "selected_switch"
            );
        }
        self.busy = false;
        match result {
            Ok(ready) => {
                log::info!(
                    request = view,
                    width = ready.area.width,
                    height = ready.area.height;
                    "source_ready"
                );
                if let InputSettings::Negative { input_floor, .. } = &mut self.document.input
                    && *input_floor == 0.
                {
                    *input_floor = crate::processing::CAMERA_INPUT_FLOOR;
                    self.document.state.project.input = self.document.input;
                }
                if self.document.visit.current_source.is_none() {
                    let mut record = ready.record.clone();
                    self.overlay_adjustment_layer_settings(
                        &mut record,
                        self.document.visit.adjustment_layer_settings,
                    );
                    record.orientation = self.document.visit.photo_orientation;
                    record.crop = self.document.visit.crop;
                    match self.document.register(record) {
                        Ok(id) => self.document.visit.current_source = Some(id),
                        Err(error) => {
                            self.status = error.to_string();
                            return;
                        }
                    }
                }
                self.status = format!(
                    "{} • {} × {}",
                    ready.path.file_name().unwrap_or_default().to_string_lossy(),
                    ready.area.width,
                    ready.area.height
                );
                if let Some(notice) = self.document.visit.return_notice.take() {
                    self.status = notice;
                }
                if let Some(id) = self.document.visit.current_source {
                    self.document.remember_source_preview(
                        id,
                        &ready.developed.metadata(),
                        ready.developed.fixed_wb(),
                    );
                }
                self.document.cache_ready(&ready);
                self.document.visit.canvas.ready = Some(*ready);
            }
            Err(error) => {
                log::warn!(
                    request = view,
                    error = error.as_str();
                    "source_load_failed"
                );

                self.document.visit.canvas.selection_timing = None;
                self.status = error;
            }
        }
    }

    // Candidate failure and Save retain the requested visit. A result that arrived
    // during preparation becomes usable without another load or another paint.
    pub(super) fn resume_source_after_document(&mut self) {
        self.resume_source_completion();
        if self.document.visit.source.pending() {
            self.busy = true;
            self.tasks.begin(Task::Photo(self.document.visit.view));
        }
    }
}
