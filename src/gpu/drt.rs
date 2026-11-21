//! The pinned ACES display rendering transform and its immutable device resources.

#[cfg(test)]
use super::{FULLSCREEN_VERTEX, fullscreen_pipeline};
use super::{aces, evaluation::*};
use crate::{Error, rendering::OutputTarget};
#[cfg(test)]
use std::borrow::Cow;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub(crate) struct Targets([Target; 2]);

impl Targets {
    pub(crate) async fn new(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
    ) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let targets = (|| {
            Ok(Self([
                target_resources(device, queue, OutputTarget::Srgb)?,
                target_resources(device, queue, OutputTarget::DisplayP3)?,
            ]))
        })();
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        targets
    }

    pub(crate) fn get(&self, target: OutputTarget) -> &Target {
        &self.0[target as usize]
    }
}

#[derive(Clone)]
pub struct Target {
    target: OutputTarget,
    math: Arc<str>,
    #[cfg(test)]
    pub pipeline: wgpu::RenderPipeline,
    #[cfg(test)]
    pub layout: wgpu::BindGroupLayout,
    pub(crate) tables: [wgpu::TextureView; 2],
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) hues: wgpu::Buffer,
}

impl Target {
    #[cfg(feature = "editor")]
    pub(crate) fn output_target(&self) -> OutputTarget {
        self.target
    }

    pub fn apply(&self, input: Value) -> Result<Value, Error> {
        let domain = input.domain();
        let code = format!(
            r#"
{}
fn pixel(cell:vec2i,position:vec2f,value:vec4f,coverage:vec4u)->vec4f {{
    if value.w<0.0 {{return value;}}
    if coverage.x!=0u && (u32(value.w)&8u)==0u {{return vec4f(0.0);}}
    if any((bitcast<vec3u>(value.rgb)&vec3u(0x7f800000u))==vec3u(0x7f800000u)) {{return vec4f(0.0,0.0,0.0,-1.0);}}
    return vec4f(antitone_render(vec4f(value.rgb,1.0)).rgb,value.w);
}}
"#,
            self.math
        );
        let bindings = vec![
            (
                1,
                FunctionBinding::Shared(Shared::Texture(self.tables[0].clone())),
            ),
            (
                2,
                FunctionBinding::Shared(Shared::Texture(self.tables[1].clone())),
            ),
            (
                3,
                FunctionBinding::Shared(Shared::Sampler(self.sampler.clone())),
            ),
            (
                5,
                FunctionBinding::Shared(Shared::Buffer(self.hues.clone())),
            ),
        ];
        let operation = value::Operation::new(
            "pinned ACES output v1",
            self.target,
            code,
            "pixel",
            value::Access::Pointwise,
        )
        .with_bindings(bindings)
        .with_uniform("vec4u", &[u32::from(domain.grid.coverage != 0), 0, 0, 0]);
        Ok(Value::from_operation(
            domain,
            Some(input),
            Display {
                target: self.target,
                operation,
            },
        ))
    }
}

struct Display {
    target: OutputTarget,
    operation: value::Operation,
}

impl Operation for Display {
    fn key(&self) -> Option<RetainedKey> {
        {
            #[derive(Debug, PartialEq, Hash)]
            struct Content(OutputTarget);
            Some(RetainedKey::new(Content(self.target)))
        }
    }

    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }

    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _budget: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        let inputs = value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            self.operation.clone(),
        )
        .map(Some)
    }
}

fn callable_library(target: OutputTarget, generated: &str) -> Result<Arc<str>, Error> {
    use std::sync::OnceLock;

    static SRGB: OnceLock<Result<Arc<str>, Error>> = OnceLock::new();

    static P3: OnceLock<Result<Arc<str>, Error>> = OnceLock::new();
    let cache = match target {
        OutputTarget::Srgb => &SRGB,
        OutputTarget::DisplayP3 => &P3,
    };
    cache.get_or_init(|| aces::library(generated)).clone()
}

fn target_assets(target: OutputTarget) -> (&'static str, [&'static [u8]; 2]) {
    match target {
        OutputTarget::Srgb => (
            include_str!("../../assets/ocio/srgb-ap1.glsl"),
            [
                include_bytes!("../../assets/ocio/srgb-ap1-lut-0.f32").as_slice(),
                include_bytes!("../../assets/ocio/srgb-ap1-lut-1.f32").as_slice(),
            ],
        ),
        OutputTarget::DisplayP3 => (
            include_str!("../../assets/ocio/display-p3-ap1.glsl"),
            [
                include_bytes!("../../assets/ocio/display-p3-ap1-lut-0.f32").as_slice(),
                include_bytes!("../../assets/ocio/display-p3-ap1-lut-1.f32").as_slice(),
            ],
        ),
    }
}

#[cfg(test)]
fn reference_target_pipeline(
    device: &wgpu::Device,
    vertex: &wgpu::ShaderModule,
    generated: &str,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let code = format!(
        r#"#version 450
layout(set=0,binding=3) uniform sampler aces_sampler;
{generated}
layout(set=0,binding=0,std430) readonly buffer Inputs {{ vec4 pixels[]; }};
layout(set=0,binding=4,std140) uniform Dimensions {{ uvec4 size; }};
layout(location=0) in vec2 uv;
layout(location=0) out vec4 result;

void main() {{
    uvec2 xy = min(uvec2(max(uv, vec2(0.0)) * vec2(size.xy)), size.xy - uvec2(1));
    vec4 pixel = pixels[xy.x + xy.y * size.x];
    if (pixel.a < 0.0) {{
        result = pixel;
        return;
    }}
    if (size.z != 0u && (uint(pixel.a) & 8u) == 0u) {{
        result = vec4(0.0);
        return;
    }}
    result = antitone_render(vec4(pixel.rgb, 1.0));
}}"#
    );
    let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pinned OCIO GLSL, resource-layout adaptation only"),
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
            buffer_entry(5, wgpu::BufferBindingType::Storage { read_only: true }),
        ],
    });
    let pipeline = fullscreen_pipeline(
        device,
        vertex,
        &fragment,
        Some(&layout),
        wgpu::TextureFormat::Rgba32Float,
    );

    (pipeline, layout)
}

fn target_resources(
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    target: OutputTarget,
) -> Result<Target, Error> {
    let (code, tables) = target_assets(target);
    let (generated, hue_values) = aces::adapt(code)?;
    #[cfg(test)]
    let (pipeline, layout) = {
        let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reference target triangle"),
            source: wgpu::ShaderSource::Wgsl(FULLSCREEN_VERTEX.into()),
        });
        reference_target_pipeline(device, &vertex, &generated)
    };
    let hues = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("immutable pinned OCIO cusp hue values"),
        contents: bytemuck::cast_slice(&hue_values),
        usage: wgpu::BufferUsages::STORAGE,
    });
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
    Ok(Target {
        target,
        math: callable_library(target, &generated)?,
        #[cfg(test)]
        pipeline,
        #[cfg(test)]
        layout,
        tables: views.try_into().unwrap(),
        sampler,
        hues,
    })
}
