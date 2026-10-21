#[cfg(test)]
#[path = "editor_look_workflow_tests.rs"]
mod tests;

use super::{
    Editor,
    look_ui::{self, Source},
};
use crate::{
    blocking_jobs::BlockingJobs,
    looks::{LookAsset, cache::Cache},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The service and any running resolver retain the cache's storage lifetime.
struct CacheOwner {
    value: Option<Cache>,
    #[cfg(test)]
    _directory: tempfile::TempDir,
}

#[derive(Clone)]
pub(super) struct Services {
    jobs: BlockingJobs,
    renderers: Arc<crate::renderer::RendererCache>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    cache: Arc<CacheOwner>,
}

impl Services {
    pub fn new(
        jobs: BlockingJobs,
        renderers: Arc<crate::renderer::RendererCache>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
    ) -> Self {
        #[cfg(test)]
        let cache = {
            let directory = tempfile::tempdir().expect("look cache fixture directory");
            CacheOwner {
                value: Some(Cache::at(directory.path().into())),
                _directory: directory,
            }
        };
        #[cfg(not(test))]
        let cache = CacheOwner {
            value: Cache::default_location(),
        };
        Self {
            jobs,
            renderers,
            device,
            queue,
            submissions,
            cache: Arc::new(cache),
        }
    }

    pub async fn resolve(
        self,
        source: Source,
        cancel: Arc<AtomicBool>,
    ) -> Result<Option<Arc<LookAsset>>, String> {
        let jobs = self.jobs.clone();
        jobs.run(move || {
            look_ui::resolve(
                source,
                &cancel,
                &self.renderers,
                &self.device,
                &self.queue,
                &self.submissions,
                self.cache.value.as_ref(),
            )
        })
        .await
        .map_err(|_| "Look preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }
}

impl Editor {
    pub(super) fn start_look_workflow(&mut self, access: &crate::application::UiAccess<'_, Self>) {
        let Some(pending) = self.document.look.as_mut() else {
            return;
        };
        let Some(source) = pending.source.take() else {
            return;
        };
        let id = pending.id;
        let cancel = pending.cancel.clone();
        let Some(services) = self.look_services.clone() else {
            self.receive_look(id, Err("Look services unavailable".into()));
            return;
        };
        let retire = Retire(cancel.clone());
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let result = services.resolve(source, cancel).await;
            target.update(|editor| editor.receive_look(id, result));
        }) else {
            self.receive_look(id, Err("Application stopped".into()));
            return;
        };
        self.document
            .look
            .as_mut()
            .expect("live look operation")
            .task = Some(task);
    }
}
