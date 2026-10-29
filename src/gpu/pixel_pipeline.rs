//! Raster adapter for the shared evaluation layer composition and declared-work executor.
use super::{
    evaluation,
    raster::{self, Raster},
};
use crate::Error;
#[cfg(test)]
use crate::pixel_pipeline::PixelPipeline;
use std::sync::Arc;

pub(crate) struct Kernels {
    #[cfg(all(test, feature = "editor"))]
    pub mtf: super::image_mtf::Kernels,
    crop: wgpu::ComputePipeline,
}
pub(crate) struct OutputRegion {
    pub size: [u32; 2],
    pub offset: [u32; 2],
}
pub(crate) struct Prepared {
    pub raster: Raster,
    execution: Option<evaluation::Recorded>,
    context: evaluation::Context,
}
impl Prepared {
    #[cfg(test)]
    pub fn new(
        device: &wgpu::Device,
        pipeline: &PixelPipeline,
        raster: Raster,
        previous: Option<&Self>,
    ) -> Result<Self, Error> {
        let context = previous.map_or_else(
            || evaluation::Context::new(device, Default::default()),
            |p| p.context.clone(),
        );
        Self::with_context(device, pipeline, raster, previous, context)
    }
    #[cfg(test)]
    pub fn with_context(
        device: &wgpu::Device,
        pipeline: &PixelPipeline,
        raster: Raster,
        previous: Option<&Self>,
        context: evaluation::Context,
    ) -> Result<Self, Error> {
        if pipeline.is_empty() {
            return Ok(Self {
                raster,
                execution: None,
                context,
            });
        }
        let domain = evaluation::Domain {
            bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
            grid: evaluation::Grid::from_raster(raster),
            meaning: evaluation::Meaning::Graded,
        };
        let root = pipeline.compose(evaluation::raster_input(domain))?;
        Self::from_composition(device, raster, root, previous, context)
    }
    pub fn from_composition(
        device: &wgpu::Device,
        raster: Raster,
        root: Arc<evaluation::ImageNode>,
        previous: Option<&Self>,
        context: evaluation::Context,
    ) -> Result<Self, Error> {
        let expected = evaluation::Domain {
            bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
            grid: evaluation::Grid::from_raster(raster),
            meaning: root.domain.meaning,
        };
        if !root.domain.same_grid(expected) {
            return Err(Error::InvalidParameter(
                "recorded composition differs from raster",
            ));
        }
        context.require_device(device)?;
        let evaluation = context.request(
            root,
            evaluation::Region {
                bounds: expected.bounds,
                grid: expected.grid,
            },
            evaluation::Budget {
                tile: raster.size(),
                scratch_bytes: device.limits().max_buffer_size.saturating_mul(4),
            },
        )?;
        let resources = previous
            .filter(|p| Arc::ptr_eq(&p.context.programs, &context.programs))
            .and_then(|p| p.execution.as_ref())
            .map(|p| p.resources())
            .unwrap_or_default();
        let execution = context.record(evaluation, resources)?;
        Ok(Self {
            raster,
            execution: Some(execution),
            context,
        })
    }
    #[cfg(all(test, feature = "editor"))]
    pub fn field(&self) -> Option<&Arc<evaluation::RetainedBuffer>> {
        self.execution.as_ref()?.cached::<super::grain::FieldKey>()
    }
    pub fn encode_from(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        destination: &wgpu::Buffer,
    ) -> Result<(), Error> {
        self.execution
            .as_ref()
            .ok_or(Error::InvalidParameter("missing recorded composition"))?
            .encode_into(device, encoder, input, destination)
    }
    pub fn submitted(&self) {
        if let Some(execution) = &self.execution {
            execution.submitted();
        }
    }
    pub fn byte_len(&self) -> u64 {
        self.execution.as_ref().map_or(0, |p| p.byte_len())
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
        #[cfg(all(test, feature = "editor"))]
        let mtf = super::image_mtf::Kernels::new(device).await?;
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
        Ok(Self {
            #[cfg(all(test, feature = "editor"))]
            mtf,
            crop,
        })
    }
    #[cfg(test)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &Prepared,
        pixels: &wgpu::Buffer,
    ) -> Result<(), Error> {
        if let Some(execution) = &prepared.execution {
            execution.encode(device, encoder, pixels)?;
        }
        Ok(())
    }
    pub fn encode_from(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        prepared: &Prepared,
        input: &wgpu::Buffer,
        destination: &wgpu::Buffer,
    ) -> Result<(), Error> {
        prepared.encode_from(device, encoder, input, destination)
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
