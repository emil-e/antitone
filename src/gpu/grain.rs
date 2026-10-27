//! Source-anchored grain stage. It consumes only pixels, raster geometry and its own model.
use super::raster::{self, Raster};
use crate::{Error, grain::GrainParameters};
use std::sync::{Arc, Mutex};

pub(crate) fn shader() -> String {
    [
        include_str!("../shaders/raster.wgsl"),
        include_str!("../shaders/grain.wgsl"),
    ]
    .join("\n")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FieldKey {
    pub raster: Raster,
    pub scale: f32,
    pub seed: u32,
}
impl FieldKey {
    pub fn new(raster: Raster, parameters: GrainParameters, seed: u64) -> Self {
        Self {
            raster,
            scale: parameters.grain_size as f32,
            seed: seed as u32 ^ ((seed >> 32) as u32).rotate_left(16),
        }
    }
}

pub(crate) struct Field {
    pub key: FieldKey,
    pub pixels: wgpu::Buffer,
    submitted: Mutex<bool>,
}
impl Field {
    pub fn prepare(
        device: &wgpu::Device,
        key: FieldKey,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        if let Some(previous) = previous.filter(|field| field.key == key) {
            return Ok(previous.clone());
        }
        Ok(Arc::new(Self {
            key,
            pixels: raster::buffer(device, key.raster.size(), "source-anchored random field")?,
            submitted: Mutex::new(false),
        }))
    }
    pub fn ready(&self) -> bool {
        *self.submitted.lock().unwrap()
    }
    pub fn submitted(&self) {
        *self.submitted.lock().unwrap() = true;
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    raster: Raster,
    values: [f32; 4],
    variance: [f32; 4],
    seed: [u32; 4],
}
pub(crate) fn parameters(
    device: &wgpu::Device,
    key: FieldKey,
    model: GrainParameters,
) -> wgpu::Buffer {
    let variance = model.channel_variance(key.raster.footprint());
    raster::uniform(
        device,
        &Parameters {
            raster: key.raster,
            values: [
                model.grain_size as f32,
                model.amplitude as f32,
                model.colour as f32,
                0.,
            ],
            variance: [
                variance[0] as f32,
                variance[1] as f32,
                variance[2] as f32,
                0.,
            ],
            seed: [key.seed, 0, 0, 0],
        },
        "grain parameters",
    )
}

pub(crate) struct Kernels {
    field: wgpu::ComputePipeline,
    apply: wgpu::ComputePipeline,
}
impl Kernels {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("source-anchored grain"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let result = Self {
            field: pipeline("field"),
            apply: pipeline("apply"),
        };
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(result)
    }
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: &wgpu::Buffer,
        field: &Field,
        pixels: &wgpu::Buffer,
    ) {
        let common = [
            wgpu::BindGroupEntry {
                binding: 10,
                resource: parameters.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: field.pixels.as_entire_binding(),
            },
        ];
        if !field.ready() {
            raster::dispatch(
                device,
                encoder,
                &self.field,
                &common,
                field.key.raster.count(),
            );
        }
        let entries = [
            wgpu::BindGroupEntry {
                binding: 1,
                resource: pixels.as_entire_binding(),
            },
            common[0].clone(),
            common[1].clone(),
        ];
        raster::dispatch(
            device,
            encoder,
            &self.apply,
            &entries,
            field.key.raster.count(),
        );
    }
}
