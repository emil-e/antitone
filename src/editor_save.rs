#[cfg(test)]
#[path = "editor_save_tests.rs"]
mod tests;

use super::{
    Editor, FailureKind, Task, TaskRequest,
    document::Document,
    document_services::{Services, Write},
};

#[derive(Default)]
pub(super) struct State {
    pub request: Option<Write>,
    task: Option<async_executor::Task<()>>,
}

impl Editor {
    pub(super) fn start_save(&mut self, write: Write) {
        self.cancel_measurement();
        self.document.history.finish_group();
        self.history_pending = None;
        let id = self.begin_document_operation(false);
        self.tasks.begin(Task::Save(id));
        self.document_failure = Some(FailureKind::Save);
        self.new_project_pending = false;
        self.document.write = State {
            request: Some(write),
            task: None,
        };
    }

    pub(super) fn start_save_workflow(&mut self, access: &crate::application::UiAccess<'_, Self>) {
        let Some(write) = self.document.write.request.take() else {
            return;
        };
        let id = self.document_pending.expect("captured save operation");
        let Some(source) = self.source_services.as_ref() else {
            self.save_result(id, Err("Document services unavailable".into()));
            return;
        };
        let services = Services {
            jobs: self.workflow_jobs.clone(),
            device: source.device.clone(),
        };
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let result = services.write(write).await.map(Box::new);
            target.update(|editor| {
                editor.save_result(id, result);
                editor.start_document_workflow(&target);
                editor.start_source_workflow(&target);
            });
        }) else {
            self.save_result(id, Err("Application stopped".into()));
            return;
        };
        self.document.write.task = Some(task);
    }

    pub(super) fn save_result(&mut self, id: u64, result: Result<Box<Document>, String>) {
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
                self.document.accept_save(*document);
                self.status = "Project saved".into();
                self.resolve_failure(FailureKind::Save);
                if let Some(transition) = self.after_save.take() {
                    self.perform_transition(transition);
                }
            }
            Err(error) => {
                self.status = format!("Save failed: {error}");
                self.transition = self.after_save.take();
                self.report_failure(FailureKind::Save, error);
            }
        }
    }
}
