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
    pub(super) fn begin_look(
        &mut self,
        source: Source,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if self.busy && self.document.look.is_none() {
            return;
        }
        self.document.look = None;
        let id = self.request_id();
        let cancel = Arc::new(AtomicBool::new(false));
        self.document.look = Some(look_ui::Pending {
            id,
            cancel: cancel.clone(),
            task: None,
        });
        self.busy = true;
        self.tasks.begin(super::Task::Look(id));
        let services = self.look_services.clone();
        let retire = Retire(cancel.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => services.resolve(source, cancel).await,
                None => Err("Look services unavailable".into()),
            };
            cx.update(|editor| editor.receive_look(id, result));
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
