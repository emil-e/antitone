//! Gaussian image-response stage. Its input is an AP1 raster, not a photographic recipe.
use super::raster::{self, Raster};
use crate::Error;

pub(crate) fn shader() -> String {
    [
        include_str!("../shaders/raster.wgsl"),
        include_str!("../shaders/image_mtf.wgsl"),
    ]
    .join("\n")
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Parameters {
    pub raster: Raster,
    pub raster_sigma: [f32; 4],
    pub radii: [u32; 4],
}
impl Parameters {
    pub fn new(raster: Raster, sigma: f64) -> Self {
        let pitch = raster.pitch().map(|value| value.max(1e-6));
        // Prepare the finite dependency once; shader arithmetic must not enlarge it.
        let support = crate::image_mtf::ImageMtfParameters { sigma }.support(pitch.map(f64::from));
        Self {
            raster,
            raster_sigma: [sigma as f32 / pitch[0], sigma as f32 / pitch[1], 0., 0.],
            radii: [support.0[0], support.0[1], 0, 0],
        }
    }
}

pub(crate) struct Kernels {
    pub horizontal: wgpu::ComputePipeline,
    vertical: wgpu::ComputePipeline,
}
impl Kernels {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Gaussian image response"),
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
            horizontal: pipeline("horizontal"),
            vertical: pipeline("vertical"),
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
        raster: Raster,
        pixels: &wgpu::Buffer,
        scratch: &wgpu::Buffer,
    ) {
        for (pipeline, input, output) in [
            (&self.horizontal, pixels, scratch),
            (&self.vertical, scratch, pixels),
        ] {
            raster::dispatch(
                device,
                encoder,
                pipeline,
                &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: parameters.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: input.as_entire_binding(),
                    },
                ],
                raster.count(),
            );
        }
    }
}
