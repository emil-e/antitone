//! Display resources and the editor's one-time foreground warm-up.

use crate::Error;
use crate::gpu::{drt::Targets, evaluation::Context, submission::Queue};
use std::sync::{Arc, LazyLock};

type LoadTargets = Box<dyn FnOnce() -> Result<Arc<Targets>, Error> + Send>;
type Prewarm = Box<dyn FnOnce() -> Result<(), Error> + Send>;
type PendingTargets = Arc<LazyLock<Result<Arc<Targets>, Error>, LoadTargets>>;

pub(crate) struct Warmup {
    targets: PendingTargets,
    foreground: LazyLock<Result<(), Error>, Prewarm>,
}

impl Warmup {
    pub fn new(context: Context, queue: Queue) -> Arc<Self> {
        let device = context.device().clone();
        let targets: PendingTargets = Arc::new(LazyLock::new(Box::new(move || {
            pollster::block_on(Targets::new(&device, &queue)).map(Arc::new)
        })));
        let pending = targets.clone();
        Arc::new(Self {
            targets,
            foreground: LazyLock::new(Box::new(move || {
                let targets = LazyLock::force(&pending).as_ref().map_err(Clone::clone)?;
                crate::photographic::graph::prewarm_viewer(&context, targets)
            })),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let warmup = self.clone();
        std::thread::spawn(move || {
            let _ = warmup.ready();
        });
    }

    pub fn targets(&self) -> Result<Arc<Targets>, Error> {
        LazyLock::force(&self.targets).clone()
    }

    pub fn ready(&self) -> Result<Arc<Targets>, Error> {
        LazyLock::force(&self.foreground)
            .as_ref()
            .map_err(Clone::clone)?;
        self.targets()
    }

    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        LazyLock::get(&self.foreground).is_some_and(Result::is_ok)
    }

    #[cfg(test)]
    pub fn failed(error: Error) -> Arc<Self> {
        let failure = error.clone();
        Arc::new(Self {
            targets: Arc::new(LazyLock::new(Box::new(move || Err(failure)))),
            foreground: LazyLock::new(Box::new(move || Err(error))),
        })
    }
}

#[cfg(test)]
pub(crate) struct DisplayFixture {
    evaluation: Context,
    looks: Arc<crate::gpu::lut3d::Looks>,
    warmup: std::sync::OnceLock<Arc<Warmup>>,
}

#[cfg(all(test, feature = "editor"))]
impl DisplayFixture {
    pub(crate) fn for_device(device: &wgpu::Device) -> Self {
        Self {
            evaluation: Context::new(device, Default::default()),
            looks: Arc::new(crate::gpu::lut3d::Looks::new(device)),
            warmup: std::sync::OnceLock::new(),
        }
    }

    pub(crate) fn evaluation(&self) -> &Context {
        &self.evaluation
    }

    pub(crate) fn looks(&self) -> &Arc<crate::gpu::lut3d::Looks> {
        &self.looks
    }

    pub(crate) fn warmup(&self, queue: &crate::gpu::submission::Queue) -> Arc<Warmup> {
        self.warmup
            .get_or_init(|| Warmup::new(self.evaluation.clone(), queue.clone()))
            .clone()
    }

    pub(crate) fn get(
        &self,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
    ) -> Result<Arc<crate::render_fixture::RenderFixture>, Error> {
        self.evaluation.require_device(device)?;
        Ok(Arc::new(crate::render_fixture::RenderFixture {
            drt: self.warmup(queue).ready()?,
            looks: self.looks.clone(),
            evaluation: self.evaluation.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_available_without_starting_foreground_warmup() {
        let (device, queue) = pollster::block_on(async {
            let adapter = crate::gpu::instance()
                .request_adapter(&Default::default())
                .await
                .unwrap();
            crate::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap()
        });
        let context = Context::new(&device, Default::default());
        let warmup = Warmup::new(context.clone(), queue);
        let targets = warmup.targets().unwrap();
        assert!(!warmup.is_ready());
        assert_eq!(context.programs.len(), 0);
        assert!(Arc::ptr_eq(&targets, &warmup.ready().unwrap()));
        assert!(warmup.is_ready());
        assert!(context.programs.len() > 0);
    }
}
