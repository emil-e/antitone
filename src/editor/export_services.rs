use super::{
    OutputFormat,
    export_batch::{self, Collision, Input, Outcome, Summary},
    source,
};
use crate::cancellation::Cancellation;
#[cfg(test)]
mod tests;
use crate::{export::Overwrite, image_recipe::ImageRecipe};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Request {
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

    pub async fn batch(
        self,
        request: export_batch::Request,
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
            let original = photo.path.clone();
            let policy = request.collision;
            let protected = request.protected.clone();
            let published = summary
                .outcomes
                .iter()
                .filter_map(|outcome| match outcome {
                    Outcome::Exported(path) => Some(path.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let planned_published = published.clone();
            let resolve = crate::background::run(move || {
                let mut photo = photo;
                let Some(path) =
                    export_batch::destination(&photo.path, policy, &protected, &planned_published)?
                else {
                    return Ok(None);
                };
                photo.path = path;
                Ok::<_, crate::Error>(Some(photo))
            })
            .await;
            let photo = match resolve {
                Ok(Ok(Some(photo))) => photo,
                Ok(Ok(None)) => {
                    summary.outcomes.push(Outcome::Skipped(original));
                    progress(summary.outcomes.len(), total);
                    continue;
                }
                failure => {
                    let fatal = failure.is_err();
                    let error = match failure {
                        Ok(Err(e)) => e.to_string(),
                        _ => "Export planning worker panicked".into(),
                    };
                    summary.outcomes.push(Outcome::Failed {
                        path: original,
                        error,
                    });
                    progress(summary.outcomes.len(), total);
                    if fatal {
                        summary.stopped = true;
                        break;
                    }
                    continue;
                }
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
                Err(error) => {
                    if cancelled.is_cancelled() {
                        summary.stopped = true;
                        break;
                    }
                    summary.outcomes.push(Outcome::Failed {
                        path: photo.path,
                        error,
                    });
                    progress(summary.outcomes.len(), total);
                    continue;
                }
            };
            let base = original;
            loop {
                if cancelled.is_cancelled() {
                    summary.stopped = true;
                    break;
                }
                let candidate = base.clone();
                let protected = request.protected.clone();
                let published = published.clone();
                let resolved = crate::background::run(move || {
                    export_batch::destination(&candidate, policy, &protected, &published)
                })
                .await;
                let path = match resolved {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) => {
                        summary.outcomes.push(Outcome::Skipped(base.clone()));
                        break;
                    }
                    failure => {
                        let fatal = failure.is_err();
                        let error = match failure {
                            Ok(Err(error)) => error.to_string(),
                            _ => "Destination check worker panicked".into(),
                        };
                        summary.outcomes.push(Outcome::Failed {
                            path: base.clone(),
                            error,
                        });
                        if fatal {
                            summary.stopped = true;
                        }
                        break;
                    }
                };
                let result = self
                    .clone()
                    .write_one(
                        Request {
                            input: input.clone(),
                            path: path.clone(),
                            format: request.format,
                            recipe: photo.recipe.clone(),
                            overwrite: if policy == Collision::Replace {
                                Overwrite::Confirmed
                            } else {
                                Overwrite::Refuse
                            },
                        },
                        cancelled.flag(),
                    )
                    .await;
                match result {
                    Ok(path) => {
                        summary.outcomes.push(Outcome::Exported(path));
                        break;
                    }
                    Err(WriteFailure::Photo(crate::Error::Cancelled)) => {
                        summary.stopped = true;
                        break;
                    }
                    Err(WriteFailure::Photo(crate::Error::DestinationExists))
                        if policy == Collision::KeepBoth =>
                    {
                        // Re-plan against the filesystem before rendering another candidate.
                        continue;
                    }
                    Err(WriteFailure::Photo(crate::Error::DestinationExists))
                        if policy == Collision::Skip =>
                    {
                        summary.outcomes.push(Outcome::Skipped(path));
                        break;
                    }
                    Err(error) => {
                        let fatal = matches!(error, WriteFailure::Infrastructure(_));
                        summary.outcomes.push(Outcome::Failed {
                            path,
                            error: error.error().to_string(),
                        });
                        if fatal {
                            summary.stopped = true;
                        }
                        break;
                    }
                }
            }
            progress(summary.outcomes.len(), total);
            if summary.stopped {
                break;
            }
        }
        summary
    }

    #[cfg(test)]
    pub async fn write(
        self,
        request: Request,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, String> {
        self.write_one(request, cancelled)
            .await
            .map_err(|error| error.error().to_string())
    }

    async fn write_one(
        self,
        request: Request,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, WriteFailure> {
        crate::background::run(move || {
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
