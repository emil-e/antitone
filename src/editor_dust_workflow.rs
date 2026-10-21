#[cfg(test)]
#[path = "editor_dust_workflow_tests.rs"]
mod tests;

use super::{Editor, dust, source};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
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

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Editor {
    pub(super) fn start_dust_workflow(&mut self, access: &crate::application::UiAccess<'_, Self>) {
        let Some((request, input)) = self.document.dust.take_job() else {
            return;
        };
        let id = request.request;
        let retire = Retire(request.cancelled.clone());
        let Some(services) = self.dust_services.clone() else {
            self.receive_dust(id, Err("Dust services unavailable".into()));
            return;
        };
        let epoch = self.source_epoch.clone();
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let result = services.evaluate(request, input, epoch).await;
            target.update(|editor| {
                editor.receive_dust(id, result);
                if let Some(signature) = editor.dust_signature() {
                    editor.schedule_dust(signature);
                    editor.start_dust_workflow(&target);
                }
            });
        }) else {
            self.receive_dust(id, Err("Application stopped".into()));
            return;
        };
        self.document.dust.attach_task(id, task);
    }
}
