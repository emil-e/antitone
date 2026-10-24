#[cfg(test)]
#[path = "editor_export_workflow_tests.rs"]
mod tests;

use super::{
    Editor, FailureKind, Task, TaskProgress, TaskRequest, UiContext,
    export_batch::{self, Outcome, Summary},
};
#[cfg(test)]
use super::{
    Error, OutputFormat, Overwrite,
    export_batch::{Collision, Input, Photo},
    export_services::Request,
};
use crate::cancellation::Cancellation;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Accepted file operations own execution independently of document/photo visits.
#[derive(Default)]
pub(super) struct State {
    pub pending: Option<u64>,
    pub cancelled: Option<Cancellation>,
    task: Option<async_executor::Task<()>>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.cancel();
        }
    }
}

impl Editor {
    #[cfg(test)]
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

    #[cfg(test)]
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
            .is_some_and(|project| super::document::same_path(project, &path))
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
        let mut protected = self
            .document
            .project
            .sources
            .iter()
            .filter_map(|s| self.document.source_path(s.id).ok())
            .collect::<Vec<_>>();
        protected.extend(self.document.path.clone());
        if let Err(error) = export_batch::require_safe(&request.path, &protected) {
            self.status = error.to_string();
            self.report_failure(FailureKind::Export, self.status.clone());
            return;
        }
        self.start_export(
            export_batch::Request {
                photos: vec![Photo {
                    input: Input::Ready(request.input),
                    recipe: request.recipe,
                    path: request.path,
                }],
                format,
                collision: if matches!(overwrite, Overwrite::Confirmed) {
                    Collision::Replace
                } else {
                    Collision::Refuse
                },
                protected,
            },
            cx,
        );
    }

    pub(super) fn start_export(
        &mut self,
        request: export_batch::Request,
        cx: &UiContext<'_, Self>,
    ) {
        let id = self.request_id();
        let cancelled = Cancellation::default();
        self.export_operation = State {
            pending: Some(id),
            cancelled: Some(cancelled.clone()),
            task: None,
        };
        self.thumbnails.cancel();
        self.busy = true;
        self.status = "Exporting…".into();
        self.export_summary = None;
        self.tasks.begin(Task::Export(id));
        let total = request.photos.len();
        log::info!(
            request = id,
            total;
            "export_started"
        );

        let services = self.export_services.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let summary = match services {
                Some(services) => {
                    services
                        .batch(request, cancelled, |done, total| {
                            cx.update(|editor| editor.export_progress(id, done, total));
                        })
                        .await
                }
                None => Summary {
                    total,
                    stopped: true,
                    outcomes: vec![Outcome::Failed {
                        path: PathBuf::new(),
                        error: "Export services unavailable".into(),
                    }],
                },
            };
            cx.update(|editor| editor.batch_result(id, summary));
        }) else {
            self.batch_result(
                id,
                Summary {
                    total,
                    stopped: true,
                    ..Default::default()
                },
            );
            return;
        };
        self.export_operation.task = Some(task);
    }

    pub(super) fn export_progress(&mut self, id: u64, completed: usize, total: usize) {
        if self.export_operation.pending != Some(id) {
            return;
        }
        self.tasks.report(
            TaskRequest::Export(id),
            TaskProgress::Counted { completed, total },
        );
    }

    pub(super) fn cancel_export(&mut self, id: u64) {
        if self.export_operation.pending != Some(id) {
            return;
        }
        if let Some(cancelled) = &self.export_operation.cancelled {
            cancelled.cancel();
        }
        log::info!(request = id; "export_cancellation_requested");
        self.tasks.cancelling(TaskRequest::Export(id));
    }

    #[cfg(test)]
    pub(super) fn export_result(&mut self, id: u64, result: Result<PathBuf, String>) {
        self.batch_result(
            id,
            Summary {
                total: 1,
                stopped: false,
                outcomes: vec![match result {
                    Ok(path) => Outcome::Exported(path),
                    Err(error) => Outcome::Failed {
                        path: PathBuf::new(),
                        error,
                    },
                }],
            },
        );
    }

    pub(super) fn batch_result(&mut self, id: u64, summary: Summary) {
        if self.export_operation.pending != Some(id) {
            return;
        }
        log::info!(
            request = id,
            total = summary.total,
            exported = summary.exported(),
            failed = summary.failed(),
            stopped = summary.stopped;
            "export_finished"
        );

        for outcome in &summary.outcomes {
            if let Outcome::Failed { error, .. } = outcome {
                log::warn!(
                    request = id,
                    error = error.as_str();
                    "export_photo_failed"
                );
            }
        }
        self.export_operation = State::default();
        if !self.tasks.finish(TaskRequest::Export(id)) {
            // A captured batch cannot finish a newer foreground operation.
            if summary.failed() > 0 && self.failure.is_none() && self.export_summary.is_none() {
                self.export_summary = Some(summary);
            }
            return;
        }
        self.busy = false;
        if summary.total == 1 && summary.exported() == 1 {
            if let Outcome::Exported(path) = &summary.outcomes[0] {
                self.status = format!("Exported {}", path.display());
            }
            self.resolve_failure(FailureKind::Export);
            self.export_summary = Some(summary);
        } else if summary.total == 1 && summary.failed() == 1 {
            if let Outcome::Failed { error, .. } = &summary.outcomes[0] {
                self.status = error.clone();
                self.report_failure(FailureKind::Export, error.clone());
            }
        } else if summary.stopped && summary.outcomes.is_empty() {
            self.status = "Export cancelled".into();
        } else {
            self.status = summary.text();
            self.export_summary = Some(summary);
        }
    }
}
