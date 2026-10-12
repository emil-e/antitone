use crate::{
    Error,
    geometry::TextureMapping,
    gpu::{GpuFlatField, ProcessingKernel, ProcessingParameters, ProcessingSource, SourceGeometry},
    rendering::OutputTarget,
};
use glam::{DMat3, Vec4};
use std::{borrow::Cow, sync::Arc};
use wgpu::util::DeviceExt;

const VERTEX: &str = r#"
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Presentation {
    columns: [Vec4; 3],
    size: [u32; 4],
    uv_x: [f32; 4],
    uv_y: [f32; 4],
}

struct ImageProcessing {
    kernel: ProcessingKernel,
    source: ProcessingSource,
    working: wgpu::Buffer,
    aces: wgpu::RenderPipeline,
    aces_group: wgpu::BindGroup,
}

pub struct GpuImage {
    last_parameters: std::sync::Mutex<Option<[u8; ProcessingParameters::BYTE_LEN]>>,
    source_revision: std::sync::atomic::AtomicU64,
    processing: Option<ImageProcessing>,
    pub linear: wgpu::Texture,
    display: wgpu::RenderPipeline,
    display_group: wgpu::BindGroup,
    presentation: Presentation,
}

impl GpuImage {
    pub async fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        pixels: &[[u16; 4]],
        camera_to_ap1: DMat3,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        if width.checked_mul(height).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::Gpu(
                "source dimensions do not match the GPU image".into(),
            ));
        }
        let source = ProcessingSource::new(device, pixels, camera_to_ap1)?;
        Self::create(device, queue, width, height, Some(source), target).await
    }

    pub async fn new_rendered(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        Self::create(device, queue, width, height, None, target).await
    }

    async fn create(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        source: Option<ProcessingSource>,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        let count = width
            .checked_mul(height)
            .filter(|n| *n > 0)
            .ok_or(Error::InvalidParameter("invalid GPU image dimensions"))?;
        if width > device.limits().max_texture_dimension_2d
            || height > device.limits().max_texture_dimension_2d
        {
            return Err(Error::Gpu("preview image exceeds GPU limits".into()));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image fullscreen triangle"),
            source: wgpu::ShaderSource::Wgsl(VERTEX.into()),
        });
        let processing = if let Some(source) = source {
            let kernel = ProcessingKernel::new(device).await?;
            let working = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("processed AP1 preview"),
                size: u64::from(count) * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
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
                generated=generated.replace(&declaration,&format!("layout(set=0, binding = {binding}) uniform texture2D {name};\n#define {name}Sampler sampler2D({name}, aces_sampler)"));
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
            let aces = create_pipeline(
                device,
                &vertex,
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
            let size = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image dimensions"),
                contents: bytemuck::cast_slice(&[width, height, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let aces_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ACES image resources"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: working.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: size.as_entire_binding(),
                    },
                ],
            });
            Some(ImageProcessing {
                kernel,
                source,
                working,
                aces,
                aces_group,
            })
        } else {
            None
        };
        let linear = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("linear target RGB, before presentation"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("target-to-extended-sRGB presentation"),
            source: wgpu::ShaderSource::Wgsl(DISPLAY.into()),
        });
        let display = create_pipeline(
            device,
            &vertex,
            &module,
            None,
            wgpu::TextureFormat::Rgba16Float,
        );
        let matrix = target.linear_to_srgb();
        let parameters = Presentation {
            columns: [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|v| v.as_vec3().extend(0.)),
            size: [width, height, 0, 0],
            uv_x: [0., 0., 1., 0.],
            uv_y: [0., 1., 0., 0.],
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("presentation basis and dimensions"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = linear.create_view(&Default::default());
        let display_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("managed image presentation"),
            layout: &display.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            last_parameters: std::sync::Mutex::new(None),
            source_revision: std::sync::atomic::AtomicU64::new(0),
            processing,
            linear,
            display,
            display_group,
            presentation: parameters,
        })
    }

    pub fn processing_byte_len(&self) -> (u64, u64) {
        self.processing
            .as_ref()
            .map_or((0, 0), |p| (p.source.byte_len(), p.working.size()))
    }

    pub fn set_flat_field(
        &mut self,
        field: Option<Arc<GpuFlatField>>,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        let processing = self
            .processing
            .as_mut()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        processing.source.set_flat_field(field, geometry)?;
        self.source_revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_parameters.lock().unwrap() = None;
        Ok(())
    }

    pub fn write_source(&self, queue: &wgpu::Queue, pixels: &[[u16; 4]]) -> Result<(), Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        processing.source.write(queue, pixels)?;
        self.source_revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_parameters.lock().unwrap() = None;
        Ok(())
    }

    pub fn source_revision(&self) -> u64 {
        self.source_revision
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Shared by viewer and scopes so either callback can prepare the whole proxy first.
    pub fn prepare(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: ProcessingParameters,
    ) -> bool {
        let signature = bytemuck::bytes_of(&parameters).try_into().unwrap();
        let mut previous = self.last_parameters.lock().unwrap();
        if *previous == Some(signature) {
            return false;
        }
        self.encode(device, encoder, parameters);
        *previous = Some(signature);
        true
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: ProcessingParameters,
    ) {
        let processing = self
            .processing
            .as_ref()
            .expect("rendered image cannot be processed");
        processing.kernel.encode(
            device,
            encoder,
            &processing.source,
            &processing.working,
            parameters,
        );
        let view = self.linear.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ACES target rendering"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&processing.aces);
        pass.set_bind_group(0, &processing.aces_group, &[]);
        pass.draw(0..3, 0..1);
    }
    pub fn presentation_group(
        &self,
        device: &wgpu::Device,
        uv: impl Into<TextureMapping>,
    ) -> wgpu::BindGroup {
        let uv = uv.into();
        let parameters = Presentation {
            uv_x: [uv.origin[0], uv.origin[1], uv.x_axis[0], uv.x_axis[1]],
            uv_y: [uv.y_axis[0], uv.y_axis[1], 0., 0.],
            ..self.presentation
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("visible source mapping"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = self.linear.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.display.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }
    pub fn paint_group(&self, pass: &mut wgpu::RenderPass<'static>, group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.display);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        self.paint_group(pass, &self.display_group);
    }
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

pub struct ImagePaint {
    pub image: Arc<GpuImage>,
    pub parameters: Option<ProcessingParameters>,
    uv: TextureMapping,
    group: std::sync::Mutex<Option<wgpu::BindGroup>>,
}
impl ImagePaint {
    pub fn new(
        image: Arc<GpuImage>,
        parameters: Option<ProcessingParameters>,
        uv: impl Into<TextureMapping>,
    ) -> Self {
        Self {
            image,
            parameters,
            uv: uv.into(),
            group: std::sync::Mutex::new(None),
        }
    }
}
impl egui_wgpu::CallbackTrait for ImagePaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(parameters) = self.parameters {
            self.image.prepare(device, encoder, parameters);
        }
        if self.uv != TextureMapping::IDENTITY {
            *self.group.lock().unwrap() = Some(self.image.presentation_group(device, self.uv));
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        match &*self.group.lock().unwrap() {
            Some(group) => self.image.paint_group(pass, group),
            None => self.image.paint(pass),
        }
    }
}
