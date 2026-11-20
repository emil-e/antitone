#[cfg(feature = "ui")]
use crate::gpu::{FULLSCREEN_VERTEX, fullscreen_pipeline};
use crate::{
    Error,
    gpu::{drt, lut3d},
};
use std::sync::Arc;

#[cfg(feature = "ui")]
const DISPLAY: &str = r#"
@group(0) @binding(0) var image: texture_2d<f32>;

struct Parameters {
    c0: vec4f,
    c1: vec4f,
    c2: vec4f,
    size: vec4u,
    uv_x: vec4f,
    uv_y: vec4f
}

@group(0) @binding(1) var<uniform> p: Parameters;

@fragment
fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
    let mapped = p.uv_x.xy + uv.x * p.uv_x.zw + uv.y * p.uv_y.xy;
    if any(mapped < vec2f(0.)) || any(mapped > vec2f(1.)) {
        discard;
    }

    let xy = min(vec2u(max(mapped, vec2f(0.)) * vec2f(p.size.xy)), p.size.xy - vec2u(1));
    let pixel = textureLoad(image, vec2i(xy), 0);
    if pixel.a <= 0. {
        discard;
    }

    let linear = mat3x3f(p.c0.xyz, p.c1.xyz, p.c2.xyz) * pixel.rgb;
    let a = abs(linear);
    let encoded = select(1.055 * pow(a, vec3f(1. / 2.4)) - .055, 12.92 * a, a <= vec3f(.0031308));
    return vec4f(sign(linear) * encoded, 1.);
}"#;

#[cfg(feature = "editor")]
#[derive(Default)]
pub(crate) struct RendererCache {
    renderer: std::sync::OnceLock<Result<Arc<GpuRenderer>, Error>>,
    submissions: crate::gpu::submission::SubmissionGate,
    retention_budget: std::sync::Mutex<Option<u64>>,
}

#[cfg(feature = "editor")]
impl RendererCache {
    pub(crate) fn new(submissions: crate::gpu::submission::SubmissionGate) -> Self {
        Self {
            submissions,
            ..Default::default()
        }
    }

    pub(crate) fn set_retention_budget(&self, bytes: u64) {
        let mut budget = self.retention_budget.lock().unwrap();
        *budget = Some(bytes);
        if let Some(Ok(renderer)) = self.renderer.get() {
            renderer.evaluation.set_retention_limit(bytes);
        }
    }

    pub(crate) fn prewarm(self: &Arc<Self>, device: wgpu::Device, queue: wgpu::Queue) {
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

    pub(crate) fn submissions(&self) -> &crate::gpu::submission::SubmissionGate {
        &self.submissions
    }

    pub(crate) fn get(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Arc<GpuRenderer>, Error> {
        let renderer = self
            .renderer
            .get_or_init(|| {
                let mut policy = crate::gpu::evaluation::CachePolicy::default();
                if let Some(bytes) = *self.retention_budget.lock().unwrap() {
                    policy.generated_bytes = bytes;
                }
                pollster::block_on(GpuRenderer::new_with_cache_policy(
                    device,
                    queue,
                    self.submissions.clone(),
                    policy,
                ))
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
    pub(crate) submissions: crate::gpu::submission::SubmissionGate,
    #[cfg(feature = "raw")]
    _pressure: crate::sys::MemoryPressureWatch,
    #[cfg(feature = "ui")]
    pub(crate) display: wgpu::RenderPipeline,
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

    pub async fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Arc<Self>, Error> {
        Self::new_with_submissions(
            device,
            queue,
            crate::gpu::submission::SubmissionGate::default(),
        )
        .await
    }

    pub async fn new_with_submissions(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: crate::gpu::submission::SubmissionGate,
    ) -> Result<Arc<Self>, Error> {
        Self::new_with_cache_policy(device, queue, submissions, Default::default()).await
    }

    pub async fn new_with_cache_policy(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: crate::gpu::submission::SubmissionGate,
        policy: crate::gpu::evaluation::CachePolicy,
    ) -> Result<Arc<Self>, Error> {
        let evaluation = crate::gpu::evaluation::Context::new(device, policy);
        #[cfg(feature = "raw")]
        let pressure = evaluation.materializations().watch_pressure();
        let looks = lut3d::Looks::new(device).await?;
        let drt = drt::Targets::new(device, queue).await?;
        #[cfg(feature = "ui")]
        let display = {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipeline = display_pipeline(device);
            if let Some(error) = scope.pop().await {
                return Err(Error::Gpu(error.to_string()));
            }
            pipeline
        };
        let renderer = Arc::new(Self {
            submissions,
            #[cfg(feature = "raw")]
            _pressure: pressure,
            #[cfg(feature = "ui")]
            display,
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

#[cfg(feature = "ui")]
pub(crate) fn display_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("image fullscreen triangle"),
        source: wgpu::ShaderSource::Wgsl(FULLSCREEN_VERTEX.into()),
    });
    let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("target-to-extended-sRGB presentation"),
        source: wgpu::ShaderSource::Wgsl(DISPLAY.into()),
    });
    fullscreen_pipeline(
        device,
        &vertex,
        &fragment,
        None,
        wgpu::TextureFormat::Rgba16Float,
    )
}
