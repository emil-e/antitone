use crate::{
    Error,
    gpu::{drt, lut3d},
};
use std::sync::Arc;

#[cfg(feature = "editor")]
#[derive(Default)]
pub(crate) struct RendererCache {
    renderer: std::sync::OnceLock<Result<Arc<GpuRenderer>, Error>>,
    retention_budget: std::sync::Mutex<Option<u64>>,
}

#[cfg(feature = "editor")]
impl RendererCache {
    pub(crate) fn set_retention_budget(&self, bytes: u64) {
        let mut budget = self.retention_budget.lock().unwrap();
        *budget = Some(bytes);
        if let Some(Ok(renderer)) = self.renderer.get() {
            renderer.evaluation.set_retention_limit(bytes);
        }
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
        let renderer = self
            .renderer
            .get_or_init(|| {
                let mut policy = crate::gpu::evaluation::CachePolicy::default();
                if let Some(bytes) = *self.retention_budget.lock().unwrap() {
                    policy.generated_bytes = bytes;
                }
                pollster::block_on(GpuRenderer::new_with_cache_policy(device, queue, policy))
            })
            .clone()?;
        // Serialize the final application with settings changes, including during initialization.
        if let Some(bytes) = *self.retention_budget.lock().unwrap() {
            renderer.evaluation.set_retention_limit(bytes);
        }
        Ok(renderer)
    }
}

/// Shared processing/rendering infrastructure for foreground images, thumbnails and refinement.
/// Image storage and processing signatures remain image-owned.
pub struct GpuRenderer {
    pub(crate) drt: drt::Targets,
    pub(crate) looks: lut3d::Looks,
    #[cfg(feature = "render")]
    pub(crate) evaluation: crate::gpu::evaluation::Context,
}

impl GpuRenderer {
    /// Shared device-scoped evaluation and its optional calculated-image retention allowance.
    pub fn evaluation(&self) -> &crate::gpu::evaluation::Context {
        &self.evaluation
    }

    pub fn looks(&self) -> &lut3d::Looks {
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
        policy: crate::gpu::evaluation::CachePolicy,
    ) -> Result<Arc<Self>, Error> {
        let evaluation = crate::gpu::evaluation::Context::new(device, policy);
        let looks = lut3d::Looks::new(device);
        let drt = drt::Targets::new(device, queue).await?;
        let renderer = Arc::new(Self {
            drt,
            looks,
            #[cfg(feature = "render")]
            evaluation,
        });
        renderer
            .evaluation
            .prewarm_texture(crate::gpu::evaluation::borrowed_input(
                crate::gpu::evaluation::Domain {
                    bounds: Some([0, 0, 1, 1].into()),
                    grid: crate::gpu::evaluation::Grid::axis_aligned([0.; 2], [1.; 2]),
                },
            ))?;
        crate::gpu::photograph::prewarm_output(&renderer.evaluation, &renderer.drt)?;
        #[cfg(feature = "editor")]
        crate::gpu::photograph::PhotographGraph::prewarm_filtered(device, &renderer)?;
        #[cfg(feature = "ui")]
        crate::gpu::photograph::PhotographGraph::prewarm_resident(device, &renderer)?;
        Ok(renderer)
    }
}
