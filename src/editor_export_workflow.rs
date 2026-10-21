#[cfg(test)]
#[path = "editor_export_workflow_tests.rs"]
mod tests;

use super::{
    Editor, Error, FailureKind, OutputFormat, Overwrite, Task, TaskRequest, UiContext, document,
    export_services::Request,
};
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
    pub pending: Option<u64>,
    pub cancelled: Option<Arc<AtomicBool>>,
    task: Option<async_executor::Task<()>>,
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
    pub(super) fn export_input(
        &self,
        path: PathBuf,
        format: OutputFormat,
        overwrite: Overwrite,
    ) -> Result<Option<Request>, Error> {
        let (Some(_), Some(ready)) = (self.calibration(), &self.document.visit.ready) else {
            return Ok(None);
        };
        self.validate_document_candidate(&self.document)?;
        let id = self
            .current_source
            .ok_or_else(|| Error::Project("no accepted source for export".into()))?;
        let recipe = self.document.project.image_recipe(id)?;
        debug_assert_eq!(recipe.composition.area(), ready.area);
        Ok(Some(Request {
            input: self.source_context().expect("ready export source"),
            path,
            format,
            recipe,
            overwrite,
        }))
    }

    pub(super) fn export(
        &mut self,
        path: PathBuf,
        format: OutputFormat,
        overwrite: Overwrite,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.auto_crop_edit.is_some() {
            self.status = "Apply or cancel auto-crop before exporting".into();
            return;
        }
        if self
            .document
            .path
            .as_ref()
            .is_some_and(|project| document::same_path(project, &path))
        {
            self.status = "Export destination is the current project document".into();
            self.report_failure(FailureKind::Export, self.status.clone());
            return;
        }
        self.sync_document();
        let request = match self.export_input(path, format, overwrite) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let id = self.request_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.export_operation = State {
            pending: Some(id),
            cancelled: Some(cancelled.clone()),
            task: None,
        };
        self.thumbnails.cancel();
        self.busy = true;
        self.status = "Exporting full-resolution source; preview pixels are not used…".into();
        self.tasks.begin(Task::Export(id));
        let services = self.export_services.clone();
        let retire = Retire(cancelled.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => services.write(request, cancelled).await,
                None => Err("Export services unavailable".into()),
            };
            cx.update(|editor| editor.export_result(id, result));
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
