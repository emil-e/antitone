use super::{Editor, Ready, Task, TaskRequest, source};
use crate::source_store::SourceKey;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
#[path = "editor_source_workflow_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct State {
    pub request: Option<source::Request>,
    task: Option<async_executor::Task<()>>,
    completion: Option<Result<Box<Ready>, String>>,
    pub key: Option<SourceKey>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.request.is_some() || self.task.is_some() || self.completion.is_some()
    }

    pub fn matches(&self, ready: &Ready) -> bool {
        self.key
            .as_ref()
            .zip(
                SourceKey::from_revision(
                    &ready.path,
                    ready.file.clone(),
                    &ready.record.development,
                    &ready.development,
                )
                .as_ref(),
            )
            .is_some_and(|(requested, prepared)| requested == prepared)
    }
}

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Editor {
    pub(super) fn start_source_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(request) = self.document.visit.source.request.take() else {
            return;
        };
        let view = self.document.visit.view;
        let Some(services) = self.source_services.clone() else {
            self.source_result(view, Err("Source services unavailable".into()));
            return;
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let retire = Retire(cancelled.clone());
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let result = services.prepare(request, cancelled).await.map(Box::new);
            target.update(|editor| editor.source_result(view, result));
        }) else {
            self.source_result(view, Err("Application stopped".into()));
            return;
        };
        self.document.visit.source.task = Some(task);
    }

    pub(super) fn source_result(&mut self, view: u64, result: Result<Box<Ready>, String>) {
        if self.document.visit.view != view {
            return;
        }
        self.document.visit.source.task = None;
        self.document.visit.source.request = None;
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
        self.pending_preview = None;
        if let Some(timing) = &self.selection_timing {
            println!(
                "selected_switch request={view} success={} boundary=source_ready elapsed_ms={:.6}",
                result.is_ok(),
                timing.started.elapsed().as_secs_f64() * 1000.
            );
        }
        self.busy = false;
        match result {
            Ok(ready) => {
                if self.floor == 0. {
                    self.floor = ready.floor;
                }
                if self.current_source.is_none() {
                    let mut record = ready.record.clone();
                    record.photo = self.photo;
                    record.orientation = self.photo_orientation;
                    record.crop = self.crop;
                    match self.document.register(record, ready.development.clone()) {
                        Ok(id) => self.current_source = Some(id),
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
                if let Some(notice) = self.return_notice.take() {
                    self.status = notice;
                }
                self.document.visit.ready = Some(*ready);
            }
            Err(error) => {
                self.selection_timing = None;
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
