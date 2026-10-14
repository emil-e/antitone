use crate::{Error, gpu::ProcessingKernel, rendering::OutputTarget, resample::AveragePipelines};
use std::{borrow::Cow, sync::Arc};

const AP1_PATCH: &str = r#"
struct Parameters { count:u32, _pad0:u32, _pad1:u32, _pad2:u32 }
@group(0) @binding(0) var<storage,read_write> image:array<vec4f>;
@group(0) @binding(1) var<storage,read> indices:array<u32>;
@group(0) @binding(2) var<storage,read> values:array<vec4f>;
@group(0) @binding(3) var<uniform> p:Parameters;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3u) {
 if id.x<p.count { image[indices[id.x]]=values[id.x]; }
}
"#;

pub(crate) const VERTEX: &str = r#"
struct Vertex { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn main(@builtin(vertex_index) i:u32) -> Vertex {
 let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.))[i];
 return Vertex(vec4f(p,0.,1.),vec2f(p.x*.5+.5,.5-p.y*.5));
}"#;
const DISPLAY: &str = r#"
@group(0) @binding(0) var image:texture_2d<f32>;
struct Parameters { c0:vec4f, c1:vec4f, c2:vec4f, size:vec4u, uv_x:vec4f, uv_y:vec4f }
@group(0) @binding(1) var<uniform> p:Parameters;
@fragment fn main(@location(0) uv:vec2f) -> @location(0) vec4f {
 let mapped=p.uv_x.xy+uv.x*p.uv_x.zw+uv.y*p.uv_y.xy;
 if any(mapped<vec2f(0.)) || any(mapped>vec2f(1.)) { discard; }
 let xy=min(vec2u(max(mapped,vec2f(0.))*vec2f(p.size.xy)),p.size.xy-vec2u(1));
 let linear=mat3x3f(p.c0.xyz,p.c1.xyz,p.c2.xyz)*textureLoad(image,vec2i(xy),0).rgb;
 let a=abs(linear);
 let encoded=select(1.055*pow(a,vec3f(1./2.4))-.055,12.92*a,a<=vec3f(.0031308));
 return vec4f(sign(linear)*encoded,1.);
}"#;

#[cfg(feature = "editor")]
#[derive(Default)]
pub(crate) struct RendererCache(std::sync::OnceLock<Result<Arc<GpuRenderer>, Error>>);
#[cfg(feature = "editor")]
impl RendererCache {
    pub(crate) fn prewarm(self: &Arc<Self>, device: wgpu::Device, queue: wgpu::Queue) {
        let cache = self.clone();
        // Initialization errors remain cached and are reported by the foreground get.
        std::thread::spawn(move || {
            let _ = cache.get(&device, &queue);
        });
    }
    pub(crate) fn get(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Arc<GpuRenderer>, Error> {
        self.0
            .get_or_init(|| pollster::block_on(GpuRenderer::new(device, queue)))
            .clone()
    }
}

pub(crate) struct TargetResources {
    pub pipeline: wgpu::RenderPipeline,
    pub layout: wgpu::BindGroupLayout,
    pub tables: [wgpu::TextureView; 2],
    pub sampler: wgpu::Sampler,
}

/// Immutable resources shared by foreground images, thumbnails and refinement.
/// Image storage and processing signatures remain image-owned.
pub struct GpuRenderer {
    pub(crate) kernel: ProcessingKernel,
    pub(crate) ap1_patch: wgpu::ComputePipeline,
    pub(crate) display: wgpu::RenderPipeline,
    targets: [TargetResources; 2],
    pub(crate) average: Arc<AveragePipelines>,
}
impl GpuRenderer {
    pub async fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Arc<Self>, Error> {
        let kernel = ProcessingKernel::new(device).await?;
        let average = Arc::new(AveragePipelines::new(device).await?);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let patch = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("AP1 repair patch"),
            source: wgpu::ShaderSource::Wgsl(AP1_PATCH.into()),
        });
        let ap1_patch = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("AP1 repair patch"),
            layout: None,
            module: &patch,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image fullscreen triangle"),
            source: wgpu::ShaderSource::Wgsl(VERTEX.into()),
        });
        let targets = [
            target_resources(device, queue, &vertex, OutputTarget::Srgb)?,
            target_resources(device, queue, &vertex, OutputTarget::DisplayP3)?,
        ];
        let display = display_pipeline(device);
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Arc::new(Self {
            kernel,
            ap1_patch,
            display,
            targets,
            average,
        }))
    }
    pub(crate) fn target(&self, target: OutputTarget) -> &TargetResources {
        &self.targets[target as usize]
    }
}

pub(crate) fn display_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("image fullscreen triangle"),
        source: wgpu::ShaderSource::Wgsl(VERTEX.into()),
    });
    let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("target-to-extended-sRGB presentation"),
        source: wgpu::ShaderSource::Wgsl(DISPLAY.into()),
    });
    create_pipeline(
        device,
        &vertex,
        &fragment,
        None,
        wgpu::TextureFormat::Rgba16Float,
    )
}

fn target_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    vertex: &wgpu::ShaderModule,
    target: OutputTarget,
) -> Result<TargetResources, Error> {
    let (code, tables) = match target {
        OutputTarget::Srgb => (
            include_str!("../assets/ocio/srgb-ap1.glsl"),
            [
                include_bytes!("../assets/ocio/srgb-ap1-lut-0.f32").as_slice(),
                include_bytes!("../assets/ocio/srgb-ap1-lut-1.f32").as_slice(),
            ],
        ),
        OutputTarget::DisplayP3 => (
            include_str!("../assets/ocio/display-p3-ap1.glsl"),
            [
                include_bytes!("../assets/ocio/display-p3-ap1-lut-0.f32").as_slice(),
                include_bytes!("../assets/ocio/display-p3-ap1-lut-1.f32").as_slice(),
            ],
        ),
    };
    let mut generated = code.to_owned();
    for (binding, name) in [(1, "aces_reach_m_table_0"), (2, "aces_gamut_cusp_table_0")] {
        let declaration =
            format!("layout(set=0, binding = {binding}) uniform sampler2D {name}Sampler;");
        if !generated.contains(&declaration) {
            return Err(Error::Gpu(
                "unexpected pinned OCIO resource declaration".into(),
            ));
        }
        generated = generated.replace(&declaration, &format!("layout(set=0, binding = {binding}) uniform texture2D {name};\n#define {name}Sampler sampler2D({name}, aces_sampler)"));
    }
    let code = format!(
        "#version 450\nlayout(set=0,binding=3) uniform sampler aces_sampler;\n{generated}\nlayout(set=0,binding=0,std430) readonly buffer Inputs {{ vec4 pixels[]; }};\nlayout(set=0,binding=4,std140) uniform Dimensions {{ uvec4 size; }};\nlayout(location=0) in vec2 uv;\nlayout(location=0) out vec4 result;\nvoid main() {{ uvec2 xy=min(uvec2(max(uv,vec2(0.0))*vec2(size.xy)),size.xy-uvec2(1)); result=antitone_render(vec4(pixels[xy.x+xy.y*size.x].rgb,1.0)); }}"
    );
    let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pinned OCIO GLSL, binding glue only"),
        source: wgpu::ShaderSource::Glsl {
            shader: Cow::Owned(code),
            stage: wgpu::naga::ShaderStage::Fragment,
            defines: Default::default(),
        },
    });
    let buffer_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        count: None,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    };
    let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        count: None,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ACES resources"),
        entries: &[
            buffer_entry(0, wgpu::BufferBindingType::Storage { read_only: true }),
            texture_entry(1),
            texture_entry(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                count: None,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
            },
            buffer_entry(4, wgpu::BufferBindingType::Uniform),
        ],
    });
    let pipeline = create_pipeline(
        device,
        vertex,
        &fragment,
        Some(&layout),
        wgpu::TextureFormat::Rgba32Float,
    );
    let sampler = device.create_sampler(&Default::default());
    let mut views = Vec::new();
    for (index, bytes) in tables.into_iter().enumerate() {
        let channels = if index == 0 { 1 } else { 3 };
        if bytes.len() != 363 * channels * 4 {
            return Err(Error::Gpu("unexpected pinned OCIO LUT size".into()));
        }
        let rgba: Vec<u8> = bytes
            .chunks_exact(channels * 4)
            .flat_map(|pixel| {
                let mut p = [0; 16];
                p[..channels * 4].copy_from_slice(pixel);
                p[12..].copy_from_slice(&1f32.to_le_bytes());
                p
            })
            .collect();
        let lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OCIO f32 table"),
            size: wgpu::Extent3d {
                width: 363,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            lut.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(363 * 16),
                rows_per_image: Some(1),
            },
            lut.size(),
        );
        views.push(lut.create_view(&Default::default()));
    }
    Ok(TargetResources {
        pipeline,
        layout,
        tables: views.try_into().unwrap(),
        sampler,
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    vertex: &wgpu::ShaderModule,
    fragment: &wgpu::ShaderModule,
    layout: Option<&wgpu::BindGroupLayout>,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = layout.map(|layout| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        })
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("image render pipeline"),
        layout: layout.as_ref(),
        vertex: wgpu::VertexState {
            module: vertex,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: fragment,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}
