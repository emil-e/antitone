use super::{OutputFormat, source};
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

#[derive(Clone)]
pub(super) struct Services {
    jobs: crate::blocking_jobs::BlockingJobs,
    renderers: Arc<crate::renderer::RendererCache>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    screening: Arc<crate::gpu::dust_screening::Cache>,
}

impl Services {
    pub fn new(
        jobs: crate::blocking_jobs::BlockingJobs,
        renderers: Arc<crate::renderer::RendererCache>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
        screening: Arc<crate::gpu::dust_screening::Cache>,
    ) -> Self {
        Self {
            jobs,
            renderers,
            device,
            queue,
            submissions,
            screening,
        }
    }

    pub async fn write(
        self,
        request: Request,
        cancelled: Arc<AtomicBool>,
    ) -> Result<PathBuf, String> {
        let jobs = self.jobs.clone();
        jobs.run(move || {
            let scan = request.input.scan()?;
            let renderer = self.renderers.get(&self.device, &self.queue)?;
            let stopped = || cancelled.load(Ordering::Relaxed);
            self.screening
                .with(&self.device, &self.queue, &self.submissions, |screener| {
                    crate::gpu_export::job::write(
                        &renderer,
                        &self.device,
                        (&self.queue, &self.submissions),
                        crate::gpu_export::job::Job {
                            scan: &scan,
                            field: request.input.correction.as_ref().map(|c| c.gpu.clone()),
                            recipe: &request.recipe,
                            format: match request.format {
                                OutputFormat::Tiff(target) => {
                                    crate::gpu_export::Format::Tiff(target)
                                }
                                OutputFormat::Exr(precision) => {
                                    crate::gpu_export::Format::Exr(precision)
                                }
                            },
                        },
                        (&request.path, request.overwrite),
                        &mut crate::dust_repair::Execution {
                            screener: Some(screener),
                            cancelled: &stopped,
                        },
                        || Ok(()),
                    )
                })?;
            Ok::<_, crate::Error>(request.path)
        })
        .await
        .map_err(|_| "Export worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }
}
