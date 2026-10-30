use super::*;
use crate::{cancellation::Cancellation, export::Overwrite, image_recipe::ImageRecipe};
use std::{
    ops::ControlFlow,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) mod batch;
#[cfg(test)]
mod tests;
mod ui;
use batch::{Collision, Input, Outcome, Summary};

#[derive(Default)]
pub(super) struct State {
    pub operation: Option<Operation>,
    pub draft: Option<batch::Draft>,
    pub summary: Option<Summary>,
}

/// The action retains cancellation feedback until its continuation acknowledges.
pub(super) struct Operation {
    pub activity: u64,
    pub cancelled: Cancellation,
    task: Option<async_executor::Task<()>>,
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

pub(super) struct PhotoRequest {
    pub input: source::Context,
    pub path: PathBuf,
    pub format: OutputFormat,
    pub recipe: ImageRecipe,
    pub overwrite: Overwrite,
}

enum WriteFailure {
    Photo(crate::Error),
    Infrastructure(crate::Error),
}

impl WriteFailure {
    fn error(&self) -> &crate::Error {
        match self {
            Self::Photo(error) | Self::Infrastructure(error) => error,
        }
    }
}

impl From<crate::Error> for WriteFailure {
    fn from(error: crate::Error) -> Self {
        Self::Photo(error)
    }
}

enum PhotoResult {
    Finished(Outcome),
    Cancelled,
    Fatal { path: PathBuf, error: String },
}

#[derive(Clone)]
pub(super) struct Services {
    renderers: Arc<crate::renderer::RendererCache>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    sources: source::Services,
}

impl Services {
    pub fn new(
        renderers: Arc<crate::renderer::RendererCache>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
        sources: source::Services,
    ) -> Self {
        Self {
            renderers,
            device,
            queue,
            submissions,
            sources,
        }
    }

    pub async fn export_batch(
        self,
        request: batch::Request,
        cancelled: Cancellation,
        mut progress: impl FnMut(usize, usize),
    ) -> Summary {
        let total = request.photos.len();
        let mut summary = Summary {
            total,
            ..Default::default()
        };
        progress(0, total);
        for photo in request.photos {
            if cancelled.is_cancelled() {
                summary.stopped = true;
                break;
            }
            let published = summary
                .outcomes
                .iter()
                .filter_map(|outcome| match outcome {
                    Outcome::Exported(path) => Some(path.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let prepared = self
                .prepare_photo(
                    photo,
                    request.format,
                    request.collision,
                    &request.protected,
                    &published,
                    &cancelled,
                )
                .await;
            let result = match prepared {
                // Borrow so this iteration retains the source through progress reporting.
                ControlFlow::Continue(ref photo) => {
                    self.export_photo(
                        photo,
                        request.collision,
                        &request.protected,
                        &published,
                        &cancelled,
                    )
                    .await
                }
                ControlFlow::Break(PhotoResult::Cancelled) => {
                    summary.stopped = true;
                    break;
                }
                ControlFlow::Break(result) => result,
            };
            match result {
                PhotoResult::Finished(outcome) => summary.outcomes.push(outcome),
                PhotoResult::Cancelled => summary.stopped = true,
                PhotoResult::Fatal { path, error } => {
                    summary.outcomes.push(Outcome::Failed { path, error });
                    summary.stopped = true;
                }
            }
            progress(summary.outcomes.len(), total);
            if summary.stopped {
                break;
            }
        }
        summary
    }

    async fn prepare_photo(
        &self,
        photo: batch::Photo,
        format: OutputFormat,
        policy: Collision,
        protected: &[PathBuf],
        published: &[PathBuf],
        cancelled: &Cancellation,
    ) -> ControlFlow<PhotoResult, PhotoRequest> {
        // Skip/protection checks precede acquisition; writes recheck afterwards.
        let path = match resolve_destination(
            &photo.path,
            policy,
            protected,
            published,
            "Export planning worker panicked",
        )
        .await
        {
            Ok(Some(path)) => path,
            Ok(None) => {
                return ControlFlow::Break(PhotoResult::Finished(Outcome::Skipped(photo.path)));
            }
            Err(result) => return ControlFlow::Break(result),
        };
        let input = match photo.input {
            Input::Ready(context) => Ok(context),
            Input::Acquire(request) => {
                self.sources
                    .clone()
                    .acquire(*request, cancelled.clone())
                    .await
            }
        };
        let input = match input {
            Ok(input) => input,
            Err(_) if cancelled.is_cancelled() => {
                return ControlFlow::Break(PhotoResult::Cancelled);
            }
            Err(error) => {
                return ControlFlow::Break(PhotoResult::Finished(Outcome::Failed { path, error }));
            }
        };
        ControlFlow::Continue(PhotoRequest {
            input,
            path: photo.path,
            format,
            recipe: photo.recipe,
            overwrite: if policy == Collision::Replace {
                Overwrite::Confirmed
            } else {
                Overwrite::Refuse
            },
        })
    }

    async fn export_photo(
        &self,
        photo: &PhotoRequest,
        policy: Collision,
        protected: &[PathBuf],
        published: &[PathBuf],
        cancelled: &Cancellation,
    ) -> PhotoResult {
        loop {
            if cancelled.is_cancelled() {
                return PhotoResult::Cancelled;
            }
            let path = match resolve_destination(
                &photo.path,
                policy,
                protected,
                published,
                "Destination check worker panicked",
            )
            .await
            {
                Ok(Some(path)) => path,
                Ok(None) => return PhotoResult::Finished(Outcome::Skipped(photo.path.clone())),
                Err(result) => return result,
            };
            let result = self
                .clone()
                .write_one(
                    PhotoRequest {
                        input: photo.input.clone(),
                        path: path.clone(),
                        format: photo.format,
                        recipe: photo.recipe.clone(),
                        overwrite: photo.overwrite,
                    },
                    cancelled.flag(),
                )
                .await;
            match result {
                Ok(path) => return PhotoResult::Finished(Outcome::Exported(path)),
                Err(WriteFailure::Photo(crate::Error::Cancelled)) => return PhotoResult::Cancelled,
                Err(WriteFailure::Photo(crate::Error::DestinationExists))
                    if policy == Collision::KeepBoth =>
                {
                    // Re-plan against the filesystem before rendering another candidate.
                    continue;
                }
                Err(WriteFailure::Photo(crate::Error::DestinationExists))
                    if policy == Collision::Skip =>
                {
                    return PhotoResult::Finished(Outcome::Skipped(path));
                }
                Err(error) => {
                    let error_text = error.error().to_string();
                    return if matches!(error, WriteFailure::Infrastructure(_)) {
                        PhotoResult::Fatal {
                            path,
                            error: error_text,
                        }
                    } else {
                        PhotoResult::Finished(Outcome::Failed {
                            path,
                            error: error_text,
                        })
                    };
                }
            }
        }
    }

    #[cfg(test)]
    pub async fn write(
        self,
        request: PhotoRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, String> {
        self.write_one(request, cancelled)
            .await
            .map_err(|error| error.error().to_string())
    }

    async fn write_one(
        self,
        request: PhotoRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, WriteFailure> {
        crate::background::run(move |_| {
            if cancelled.load(Ordering::Relaxed) {
                return Err(WriteFailure::Photo(crate::Error::Cancelled));
            }
            let scan = request.input.scan()?;
            let renderer = self
                .renderers
                .get(&self.device, &self.queue)
                .map_err(WriteFailure::Infrastructure)?;
            let stopped = || cancelled.load(Ordering::Relaxed);
            crate::gpu_export::job::write(
                &renderer,
                &self.device,
                (&self.queue, &self.submissions),
                crate::gpu_export::job::Job {
                    scan: &scan,
                    field: request.input.correction.as_ref().map(|c| c.gpu.clone()),
                    recipe: &request.recipe,
                    format: match request.format {
                        OutputFormat::Tiff(target) => crate::gpu_export::Format::Tiff(target),
                        OutputFormat::Exr(precision) => crate::gpu_export::Format::Exr(precision),
                    },
                },
                (&request.path, request.overwrite),
                &mut crate::dust_repair::Execution {
                    cancelled: &stopped,
                },
                || Ok(()),
            )?;
            Ok::<_, WriteFailure>(request.path)
        })
        .await
        .map_err(|_| {
            WriteFailure::Infrastructure(crate::Error::Gpu("Export worker panicked".to_owned()))
        })?
    }
}

async fn resolve_destination(
    path: &Path,
    policy: Collision,
    protected: &[PathBuf],
    published: &[PathBuf],
    panic_message: &'static str,
) -> Result<Option<PathBuf>, PhotoResult> {
    let failed_path = path.to_owned();
    let candidate = path.to_owned();
    let protected = protected.to_vec();
    let published = published.to_vec();
    match crate::background::run(move |_| {
        batch::destination(&candidate, policy, &protected, &published)
    })
    .await
    {
        Ok(Ok(path)) => Ok(path),
        Ok(Err(error)) => Err(PhotoResult::Finished(Outcome::Failed {
            path: failed_path,
            error: error.to_string(),
        })),
        Err(_) => Err(PhotoResult::Fatal {
            path: failed_path,
            error: panic_message.into(),
        }),
    }
}

impl Editor {
    pub(super) fn export_photos(&mut self, request: batch::Request, cx: &UiContext<'_, Self>) {
        let activity = self.request_id();
        let cancelled = Cancellation::default();
        self.document.export.operation = Some(Operation {
            activity,
            cancelled: cancelled.clone(),
            task: None,
        });
        self.document.thumbnails.cancel();
        self.busy = true;
        self.status = "Exporting…".into();
        self.document.export.summary = None;
        self.tasks.begin(Task::Export(activity));
        let total = request.photos.len();
        log::info!(request = activity, total; "export_started");

        let services = self.export_services.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let summary = match services {
                Some(services) => {
                    services
                        .export_batch(request, cancelled, |completed, total| {
                            cx.update(|editor| {
                                editor.tasks.report(
                                    TaskRequest::Export(activity),
                                    TaskProgress::Counted { completed, total },
                                )
                            });
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
            cx.update(|editor| {
                log::info!(
                    request = activity,
                    total = summary.total,
                    exported = summary.exported(),
                    failed = summary.failed(),
                    stopped = summary.stopped;
                    "export_finished"
                );

                for outcome in &summary.outcomes {
                    if let Outcome::Failed { error, .. } = outcome {
                        log::warn!(
                            request = activity,
                            error = error.as_str();
                            "export_photo_failed"
                        );
                    }
                }
                editor.document.export.operation = None;
                if !editor.tasks.finish(TaskRequest::Export(activity)) {
                    // A captured batch cannot finish a newer foreground operation.
                    if summary.failed() > 0
                        && editor.failure.is_none()
                        && editor.document.export.summary.is_none()
                    {
                        editor.document.export.summary = Some(summary);
                    }
                    return;
                }
                editor.busy = false;
                if summary.total == 1 && summary.exported() == 1 {
                    if let Outcome::Exported(path) = &summary.outcomes[0] {
                        editor.status = format!("Exported {}", path.display());
                    }
                    editor.resolve_failure(FailureKind::Export);
                    editor.document.export.summary = Some(summary);
                } else if summary.total == 1 && summary.failed() == 1 {
                    if let Outcome::Failed { error, .. } = &summary.outcomes[0] {
                        editor.status = error.clone();
                        editor.report_failure(FailureKind::Export, error.clone());
                    }
                } else if summary.stopped && summary.outcomes.is_empty() {
                    editor.status = "Export cancelled".into();
                } else {
                    editor.status = summary.text();
                    editor.document.export.summary = Some(summary);
                }
            });
        }) else {
            self.document.export.operation = None;
            self.tasks.finish(TaskRequest::Export(activity));
            self.busy = false;
            self.status = "Export cancelled".into();
            return;
        };
        self.document.export.operation.as_mut().unwrap().task = Some(task);
    }

    pub(super) fn cancel_export(&mut self) {
        let Some(operation) = &self.document.export.operation else {
            return;
        };
        operation.cancelled.cancel();
        log::info!(request = operation.activity; "export_cancellation_requested");
        self.tasks
            .cancelling(TaskRequest::Export(operation.activity));
    }
}
