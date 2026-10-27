//! Pixel-stage executor: ordering, transient scratch, cache reuse and output extraction.
use super::{
    grain, image_mtf,
    raster::{self, Raster},
};
use crate::{
    Error,
    pixel_pipeline::{PixelPipeline, PixelStage},
};
use std::sync::Arc;

pub(crate) struct Kernels {
    pub mtf: image_mtf::Kernels,
    grain: grain::Kernels,
    crop: wgpu::ComputePipeline,
}
enum Stage {
    Gaussian {
        uniform: wgpu::Buffer,
    },
    Grain {
        uniform: wgpu::Buffer,
        field: Arc<grain::Field>,
        active: bool,
    },
}
pub(crate) struct OutputRegion {
    pub size: [u32; 2],
    pub offset: [u32; 2],
}

pub(crate) struct Prepared {
    pub raster: Raster,
    stages: Vec<Stage>,
}

impl Prepared {
    pub fn new(
        device: &wgpu::Device,
        pipeline: &PixelPipeline,
        raster: Raster,
        previous: Option<&Self>,
    ) -> Result<Self, Error> {
        let mut stages = Vec::new();
        for stage in &pipeline.stages {
            stages.push(match *stage {
                PixelStage::Gaussian { sigma } => Stage::Gaussian {
                    uniform: raster::uniform(
                        device,
                        &image_mtf::Parameters::new(raster, sigma),
                        "image-response parameters",
                    ),
                },
                PixelStage::Grain { parameters, seed } => {
                    let key = grain::FieldKey::new(raster, parameters, seed);
                    let cached = previous.and_then(Self::field);
                    let field = grain::Field::prepare(device, key, cached)?;
                    Stage::Grain {
                        uniform: grain::parameters(device, key, parameters),
                        field,
                        active: parameters.amplitude > 0.,
                    }
                }
            });
        }
        Ok(Self { raster, stages })
    }
    pub fn field(&self) -> Option<&Arc<grain::Field>> {
        self.stages.iter().find_map(|stage| {
            if let Stage::Grain { field, .. } = stage {
                Some(field)
            } else {
                None
            }
        })
    }
    pub fn submitted(&self) {
        for stage in &self.stages {
            if let Stage::Grain {
                field,
                active: true,
                ..
            } = stage
            {
                field.submitted();
            }
        }
    }
    pub fn byte_len(&self) -> u64 {
        self.field().map_or(0, |field| field.pixels.size())
    }
    pub fn input_buffer(
        &self,
        device: &wgpu::Device,
        output: [u32; 2],
    ) -> Result<Option<wgpu::Buffer>, Error> {
        if self.raster.size() == output {
            Ok(None)
        } else {
            raster::buffer(device, self.raster.size(), "bounded processing input").map(Some)
        }
    }
}

impl Kernels {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let mtf = image_mtf::Kernels::new(device).await?;
        let grain = grain::Kernels::new(device).await?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster output extraction"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/raster_crop.wgsl").into()),
        });
        let crop = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self { mtf, grain, crop })
    }
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &Prepared,
        pixels: &wgpu::Buffer,
    ) -> Result<(), Error> {
        let scratch = if prepared
            .stages
            .iter()
            .any(|stage| matches!(stage, Stage::Gaussian { .. }))
        {
            Some(raster::buffer(
                device,
                prepared.raster.size(),
                "pixel-stage scratch",
            )?)
        } else {
            None
        };
        for stage in &prepared.stages {
            match stage {
                Stage::Gaussian { uniform } => self.mtf.encode(
                    device,
                    encoder,
                    uniform,
                    prepared.raster,
                    pixels,
                    scratch.as_ref().unwrap(),
                ),
                Stage::Grain {
                    uniform,
                    field,
                    active: true,
                } => self.grain.encode(device, encoder, uniform, field, pixels),
                Stage::Grain { active: false, .. } => {}
            }
        }
        Ok(())
    }
    pub fn extract(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        input_size: [u32; 2],
        output: &wgpu::Buffer,
        region: OutputRegion,
    ) {
        let OutputRegion {
            size: output_size,
            offset,
        } = region;
        let values = [
            input_size[0],
            input_size[1],
            output_size[0],
            output_size[1],
            offset[0],
            offset[1],
            0,
            0,
        ];
        let uniform = raster::uniform(device, &values, "valid output region");
        raster::dispatch(
            device,
            encoder,
            &self.crop,
            &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: input.as_entire_binding(),
                },
            ],
            output_size[0] * output_size[1],
        );
    }
}
