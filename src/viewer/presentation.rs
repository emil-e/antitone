//! Draw a target-linear texture into the viewer's extended-sRGB attachment.

use crate::{
    Error,
    geometry::TextureMapping,
    gpu::{FULLSCREEN_VERTEX, allocation::DeviceAllocationExt, fullscreen_pipeline},
    rendering::OutputTarget,
};
use glam::Vec4;

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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    columns: [Vec4; 3],
    size: [u32; 4],
    uv_x: [f32; 4],
    uv_y: [f32; 4],
}

/// Shared by the canvas and filmstrip on one device.
pub struct Presentation {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Presentation {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = pipeline(device);
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            device: device.clone(),
            layout: pipeline.get_bind_group_layout(0),
            pipeline,
        })
    }

    pub fn bind(
        &self,
        image: &wgpu::Texture,
        target: OutputTarget,
        mapping: impl Into<TextureMapping>,
    ) -> Result<Draw, Error> {
        let mapping = mapping.into();
        let matrix = target.linear_to_srgb();
        let parameters = Parameters {
            columns: [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|v| v.as_vec3().extend(0.)),
            size: [image.width(), image.height(), 0, 0],
            uv_x: [
                mapping.origin[0],
                mapping.origin[1],
                mapping.x_axis[0],
                mapping.x_axis[1],
            ],
            uv_y: [mapping.y_axis[0], mapping.y_axis[1], 0., 0.],
        };
        let group = make_presentation_group(&self.device, &self.layout, image, parameters)?;
        Ok(Draw {
            pipeline: self.pipeline.clone(),
            group,
            mapping,
        })
    }
}

/// A mapped image bound to the pipeline that can draw it.
#[derive(Clone)]
pub struct Draw {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    mapping: TextureMapping,
}

impl Draw {
    pub(super) fn mapping(&self) -> TextureMapping {
        self.mapping
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Keep the identity binding and at most the last mapped binding for this texture.
#[derive(Default)]
pub(super) struct Bindings {
    identity: Option<Draw>,
    mapped: Option<Draw>,
}

impl Bindings {
    pub(super) fn prepare(
        &mut self,
        presentation: &Presentation,
        image: &wgpu::Texture,
        target: OutputTarget,
        mapping: TextureMapping,
    ) -> Result<Draw, Error> {
        if mapping == TextureMapping::IDENTITY {
            if let Some(draw) = &self.identity {
                return Ok(draw.clone());
            }
            let draw = presentation.bind(image, target, mapping)?;
            self.identity = Some(draw.clone());
            return Ok(draw);
        }

        if let Some(draw) = &self.mapped
            && draw.mapping() == mapping
        {
            return Ok(draw.clone());
        }
        let draw = presentation.bind(image, target, mapping)?;
        self.mapped = Some(draw.clone());
        Ok(draw)
    }

    #[cfg(feature = "editor")]
    pub(super) fn retained(&self) -> Option<Draw> {
        self.mapped.as_ref().or(self.identity.as_ref()).cloned()
    }
}

fn make_presentation_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    linear: &wgpu::Texture,
    parameters: Parameters,
) -> Result<wgpu::BindGroup, Error> {
    let uniform = device.try_create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("presentation basis, dimensions and mapping"),
        contents: bytemuck::bytes_of(&parameters),
        usage: wgpu::BufferUsages::UNIFORM,
    })?;
    crate::gpu::allocation::checked(device, || {
        let view = linear.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("managed image presentation"),
            layout,
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
    })
}

fn pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
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
