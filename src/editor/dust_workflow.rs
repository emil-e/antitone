#[cfg(test)]
mod tests;

#[cfg(test)]
use super::Editor;
use super::{dust, source};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub(super) struct Services {
    jobs: crate::blocking_jobs::BlockingJobs,
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    screening: Arc<crate::gpu::dust_screening::Cache>,
}

impl Services {
    pub fn new(
        jobs: crate::blocking_jobs::BlockingJobs,
        device: wgpu::Device,
        queue: wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
        screening: Arc<crate::gpu::dust_screening::Cache>,
    ) -> Self {
        Self {
            jobs,
            device,
            queue,
            submissions,
            screening,
        }
    }

    pub async fn evaluate(
        self,
        request: dust::Request,
        input: source::Context,
        source_epoch: Arc<AtomicU64>,
    ) -> Result<Option<dust::Response>, String> {
        let jobs = self.jobs.clone();
        jobs.run(move || {
            let cancelled = || {
                if source_epoch.load(Ordering::Relaxed) != request.loaded {
                    request.cancelled.store(true, Ordering::Relaxed);
                }
                request.cancelled.load(Ordering::Relaxed)
            };
            if cancelled() {
                return Ok(None);
            }
            let result = input.scan().and_then(|scan| {
                self.screening
                    .with(&self.device, &self.queue, &self.submissions, |screening| {
                        dust::evaluate_with(&scan, &request, Some(screening), &cancelled)
                    })
            });
            if cancelled() { Ok(None) } else { result }
        })
        .await
        .map_err(|_| "Dust evaluation worker panicked".to_owned())?
        .map_err(|error: crate::Error| error.to_string())
    }
}
