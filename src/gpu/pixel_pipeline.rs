//! Raster adapter for the shared pull layer composition and declared-work executor.
use super::{
    pull,
    raster::{self, Raster},
};
use crate::{Error, pixel_pipeline::PixelPipeline};
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
    execution: Option<pull::Recorded>,
}
impl Prepared {
    #[cfg(test)]
    pub fn new(
        device: &wgpu::Device,
        pipeline: &PixelPipeline,
        raster: Raster,
        previous: Option<&Self>,
    ) -> Result<Self, Error> {
        Self::with_programs(device, pipeline, raster, previous, Arc::default())
    }
    pub fn with_programs(
        device: &wgpu::Device,
        pipeline: &PixelPipeline,
        raster: Raster,
        previous: Option<&Self>,
        programs: Arc<pull::Programs>,
    ) -> Result<Self, Error> {
        let execution = if pipeline.is_empty() {
            None
        } else {
            let domain = pull::Domain {
                bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
                grid: pull::Grid::from_raster(raster),
                meaning: pull::Meaning::Graded,
            };
            let input = pull::raster_input(domain);
            let root = pipeline.compose(input)?;
            let plan = pull::Plan::with_policy(
                root,
                domain.bounds,
                pull::Budget {
                    tile: raster.size(),
                    scratch_bytes: device.limits().max_buffer_size.saturating_mul(4),
                },
                pull::Policy::Composed,
            )?;
            let resources = previous
                .and_then(|p| p.execution.as_ref())
                .map(|p| p.resources())
                .unwrap_or_default();
            Some(pull::Executor::with_programs(programs).prepare(device, plan, resources)?)
        };
        Ok(Self { raster, execution })
    }
    #[cfg(all(test, feature = "editor"))]
    pub fn field(&self) -> Option<&Arc<pull::RetainedBuffer>> {
        self.execution.as_ref()?.cached::<super::grain::FieldKey>()
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
