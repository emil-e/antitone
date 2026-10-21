#[cfg(test)]
#[path = "editor_export_workflow_tests.rs"]
mod tests;

use super::{Editor, FailureKind, TaskRequest, export_services::Request};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// An accepted file operation is independent of the visible document/grade visit.
#[derive(Default)]
pub(super) struct State {
    pub request: Option<Request>,
    pub pending: Option<u64>,
    pub cancelled: Option<Arc<AtomicBool>>,
    task: Option<async_executor::Task<()>>,
}

impl State {
    pub fn new(id: u64, request: Request) -> Self {
        Self {
            request: Some(request),
            pending: Some(id),
            cancelled: Some(Arc::new(AtomicBool::new(false))),
            task: None,
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::Relaxed);
        }
    }
}

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Editor {
    pub(super) fn start_export_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(request) = self.export_operation.request.take() else {
            return;
        };
        let id = self
            .export_operation
            .pending
            .expect("captured export operation");
        let Some(services) = self.export_services.clone() else {
            self.export_result(id, Err("Export services unavailable".into()));
            return;
        };
        let cancelled = self.export_operation.cancelled.as_ref().unwrap().clone();
        let retire = Retire(cancelled.clone());
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let result = services.write(request, cancelled).await;
            target.update(|editor| editor.export_result(id, result));
        }) else {
            self.export_result(id, Err("Application stopped".into()));
            return;
        };
        self.export_operation.task = Some(task);
    }

    pub(super) fn export_result(&mut self, id: u64, result: Result<PathBuf, String>) {
        if self.export_operation.pending != Some(id) {
            return;
        }
        self.export_operation = State::default();
        if !self.tasks.finish(TaskRequest::Export(id)) {
            // A captured file can finish after a newer operation takes foreground ownership.
            if let Err(error) = result
                && self.failure.is_none()
            {
                self.report_failure(FailureKind::Export, error);
            }
            return;
        }
        self.busy = false;
        match result {
            Ok(path) => {
                self.status = format!("Exported {}", path.display());
                self.resolve_failure(FailureKind::Export);
            }
            Err(error) => {
                self.status = error.clone();
                self.report_failure(FailureKind::Export, error);
            }
        }
    }
}
