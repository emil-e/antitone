#[cfg(test)]
mod tests;

use super::{
    Editor, FailureKind, Task, TaskRequest, UiContext,
    document::Document,
    document_services::{Services, Write},
};

#[derive(Default)]
pub(super) struct State {
    task: Option<async_executor::Task<()>>,
}

impl Editor {
    pub(super) fn start_save(&mut self, write: Write, cx: &UiContext<'_, Self>) {
        self.cancel_measurement();
        self.document.history.finish_group();
        self.history_pending = None;
        let id = self.begin_document_operation(false);
        self.tasks.begin(Task::Save(id));
        self.document_failure = Some(FailureKind::Save);
        self.new_project_pending = false;
        let services = self.source_services.as_ref().map(|source| Services {
            jobs: self.workflow_jobs.clone(),
            device: source.device.clone(),
        });
        let Some(task) = cx.spawn(move |cx| async move {
            let result = match services {
                Some(services) => services.write(write).await.map(Box::new),
                None => Err("Document services unavailable".into()),
            };
            cx.update(|editor| {
                editor.save_result(id, result, &cx);
                editor.acquire_working(&cx);
            });
        }) else {
            self.save_result(id, Err("Application stopped".into()), cx);
            return;
        };
        self.document.write.task = Some(task);
    }

    pub(super) fn save_result(
        &mut self,
        id: u64,
        result: Result<Box<Document>, String>,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document_pending != Some(id) {
            return;
        }
        self.document.write = State::default();
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.tasks.finish(TaskRequest::Document(id));
        self.document_pending = None;
        self.document_failure = None;
        self.busy = false;
        self.resume_source_after_document();
        match result {
            Ok(document) => {
                log::info!(request = id; "project_saved");
                self.document.accept_save(*document);
                self.status = "Project saved".into();
                self.resolve_failure(FailureKind::Save);
                if let Some(transition) = self.after_save.take() {
                    self.perform_transition(transition, cx);
                }
            }
            Err(error) => {
                self.status = format!("Save failed: {error}");
                self.transition = self.after_save.take();
                self.report_failure(FailureKind::Save, error);
            }
        }
        if self.document_pending.is_none()
            && let Some(pending) = self.document.spatial_preparing.as_ref()
        {
            self.tasks.begin(Task::Regions(pending.id));
            self.busy = true;
            self.status = "Preparing film and picture regions…".into();
        } else {
            self.resume_spatial_consumer(cx);
        }
    }
}
