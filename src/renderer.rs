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

pub(crate) struct RenderBinding {
    source: crate::gpu::evaluation::SuppliedImage,
    looks: Vec<Arc<lut3d::Lut>>,
    recorded: crate::gpu::evaluation::Recorded,
    destination: wgpu::Texture,
    validation: Option<Arc<crate::gpu::lut3d::Validation>>,
}

impl RenderBinding {
    pub(crate) fn look_id(&self) -> Option<crate::looks::LookId> {
        crate::looks::LutChain::ordered_id(self.looks.iter().map(|lut| lut.id()))
    }

    pub(crate) fn retained_bytes(&self) -> u64 {
        self.looks.iter().map(|lut| lut.byte_len()).sum::<u64>() + self.recorded.byte_len()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn look_resources(&self) -> Vec<Arc<lut3d::Lut>> {
        self.looks.clone()
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Option<Arc<crate::gpu::lut3d::Check>>, Error> {
        self.recorded
            .encode_work(device, encoder, &[self.source.pixels()])?;
        Ok(self
            .validation
            .as_ref()
            .map(|validation| validation.check_rendered(device, encoder, &self.destination)))
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
        let looks = lut3d::Looks::new(device).await?;
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
                    meaning: crate::gpu::evaluation::Meaning::Display,
                },
            ))?;
        crate::gpu::photograph::prewarm_output(&renderer.evaluation, &renderer.drt)?;
        #[cfg(feature = "editor")]
        crate::gpu::photograph::PhotographGraph::prewarm_filtered(device, &renderer)?;
        #[cfg(feature = "ui")]
        crate::gpu::photograph::PhotographGraph::prewarm_resident(device, &renderer)?;
        Ok(renderer)
    }

    pub(crate) fn render_binding(
        &self,
        device: &wgpu::Device,
        source: crate::gpu::evaluation::SuppliedImage,
        image: crate::gpu::evaluation::Value,
        destination: &wgpu::Texture,
    ) -> Result<RenderBinding, Error> {
        use crate::gpu::evaluation::{ImageFormat, Region, Tiling};
        self.evaluation.require_device(device)?;
        let domain = source.definition().domain();
        let region = source.layout().region;
        if source.layout().format != ImageFormat::Rgba32Float {
            return Err(Error::InvalidParameter("invalid rendered source storage"));
        }
        if image.domain().meaning != crate::gpu::evaluation::Meaning::Display {
            return Err(Error::InvalidParameter(
                "target rendering requires a display endpoint",
            ));
        }
        let looks = lut3d::applied_luts(&image);
        let validation = (!looks.is_empty()).then(|| self.looks.validation());
        let evaluation = self.evaluation.request(
            image,
            Region {
                bounds: region,
                grid: domain.grid,
            },
            Tiling {
                tile: [region.width, region.height],
            },
        )?;
        let recorded = self.evaluation.record_to_texture(
            evaluation,
            self.evaluation.materializations(),
            vec![source.clone()],
            destination,
        )?;
        Ok(RenderBinding {
            source,
            looks,
            recorded,
            destination: destination.clone(),
            validation,
        })
    }
}
