use crate::{
    Error,
    gpu::{
        drt,
        evaluation::{CachePolicy, Context},
        lut3d,
    },
};
use std::sync::Arc;

#[cfg(feature = "editor")]
pub(crate) struct RendererCache {
    evaluation: Context,
    renderer: std::sync::OnceLock<Result<Arc<GpuRenderer>, Error>>,
}

#[cfg(feature = "editor")]
impl RendererCache {
    pub(crate) fn new(evaluation: Context) -> Self {
        Self {
            evaluation,
            renderer: std::sync::OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_device(device: &wgpu::Device) -> Self {
        Self::new(Context::new(device, CachePolicy::default()))
    }

    pub(crate) fn evaluation(&self) -> &Context {
        &self.evaluation
    }

    pub(crate) fn prewarm(
        self: &Arc<Self>,
        device: wgpu::Device,
        queue: crate::gpu::submission::Queue,
    ) {
        let cache = self.clone();
        // Initialization errors remain cached and are reported by foreground consumers.
        std::thread::spawn(move || {
            let _ = cache.get(&device, &queue);
        });
    }

    #[cfg(test)]
    pub(crate) fn fail_initialization(&self, error: Error) {
        assert!(
            self.renderer.set(Err(error)).is_ok(),
            "fixture requires an uninitialized renderer cache"
        );
    }

    #[cfg(test)]
    pub(crate) fn is_initialized(&self) -> bool {
        self.renderer.get().is_some()
    }

    pub(crate) fn get(
        &self,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
    ) -> Result<Arc<GpuRenderer>, Error> {
        self.evaluation.require_device(device)?;
        self.renderer
            .get_or_init(|| {
                pollster::block_on(GpuRenderer::with_evaluation(self.evaluation.clone(), queue))
            })
            .clone()
    }
}

/// Shared processing/rendering infrastructure for foreground images, thumbnails and refinement.
/// Image storage and processing signatures remain image-owned.
pub struct GpuRenderer {
    pub(crate) drt: Arc<drt::Targets>,
    pub(crate) looks: Arc<lut3d::Looks>,
    #[cfg(feature = "render")]
    pub(crate) evaluation: crate::gpu::evaluation::Context,
}

impl GpuRenderer {
    /// Shared device-scoped evaluation and its optional calculated-image retention allowance.
    pub fn evaluation(&self) -> &crate::gpu::evaluation::Context {
        &self.evaluation
    }

    pub fn looks(&self) -> &Arc<lut3d::Looks> {
        &self.looks
    }

    pub fn drt(&self, target: crate::rendering::OutputTarget) -> &drt::Target {
        self.drt.get(target)
    }

    pub async fn new(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
    ) -> Result<Arc<Self>, Error> {
        Self::new_with_cache_policy(device, queue, Default::default()).await
    }

    pub async fn new_with_cache_policy(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        policy: CachePolicy,
    ) -> Result<Arc<Self>, Error> {
        Self::with_evaluation(Context::new(device, policy), queue).await
    }

    async fn with_evaluation(
        evaluation: Context,
        queue: &crate::gpu::submission::Queue,
    ) -> Result<Arc<Self>, Error> {
        let device = evaluation.device().clone();
        let looks = Arc::new(lut3d::Looks::new(&device));
        let drt = Arc::new(drt::Targets::new(&device, queue).await?);
        let renderer = Arc::new(Self {
            drt,
            looks,
            #[cfg(feature = "render")]
            evaluation,
        });
        crate::gpu::photograph::prewarm_viewer(&renderer.evaluation, &renderer.drt)?;
        Ok(renderer)
    }
}
