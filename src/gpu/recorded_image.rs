//! Raster adapter for declared GPU work over an explicit image value.

use super::{evaluation, raster::Raster};
use crate::Error;
#[cfg(test)]
use crate::photographic::graph::authoring::Assembly;

pub(crate) struct Prepared {
    execution: Option<evaluation::Recorded>,
}

impl Prepared {
    #[cfg(test)]
    pub fn new(device: &wgpu::Device, pipeline: &Assembly, raster: Raster) -> Result<Self, Error> {
        Self::with_context(
            device,
            pipeline,
            raster,
            evaluation::Context::new(device, Default::default()),
        )
    }

    #[cfg(test)]
    pub fn with_context(
        device: &wgpu::Device,
        pipeline: &Assembly,
        raster: Raster,
        context: evaluation::Context,
    ) -> Result<Self, Error> {
        let domain = evaluation::Domain {
            bounds: Some(evaluation::CellRegion::new([0, 0], raster.size())),
            grid: evaluation::Grid::from_raster(raster),
        };
        let root = pipeline.processed(evaluation::raster_input(domain), None)?;
        if pipeline.is_empty() {
            return Ok(Self { execution: None });
        }
        Self::from_composition(device, raster, root, context)
    }

    pub fn from_composition(
        device: &wgpu::Device,
        mut raster: Raster,
        root: evaluation::Value,
        context: evaluation::Context,
    ) -> Result<Self, Error> {
        context.require_device(device)?;
        raster.dimensions[2] = root.domain().grid.coverage;
        let expected = evaluation::Domain {
            bounds: Some(evaluation::CellRegion::new([0, 0], raster.size())),
            grid: evaluation::Grid::from_raster(raster),
        };
        if root.domain().bounds != expected.bounds || !root.domain().same_grid(expected) {
            return Err(Error::InvalidParameter(
                "recorded composition differs from raster",
            ));
        }
        let region = expected.finite()?;
        let request = context.request(
            root,
            evaluation::Region {
                bounds: region,
                grid: expected.grid,
            },
            evaluation::Tiling {
                tile: [region.width, region.height],
            },
        )?;
        let execution = context.record(request, context.materializations(), Vec::new())?;
        Ok(Self {
            execution: Some(execution),
        })
    }

    #[cfg(test)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pixels: &wgpu::Buffer,
    ) -> Result<(), Error> {
        if let Some(execution) = &self.execution {
            execution.encode(device, encoder, pixels)?;
        }
        Ok(())
    }

    #[cfg(test)]
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

    pub(crate) fn encode_result(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Buffer,
    ) -> Result<(), Error> {
        self.execution
            .as_ref()
            .ok_or(Error::InvalidParameter("missing recorded composition"))?
            .encode_result(device, encoder, destination)
    }
}

/// Prepared recording from a supplied image into a resident float texture.
#[cfg(feature = "render")]
pub(crate) struct Texture {
    source: evaluation::SuppliedImage,
    endpoint: evaluation::Value,
    recorded: evaluation::Recorded,
}

#[cfg(feature = "render")]
impl Texture {
    pub(crate) fn new(
        context: &evaluation::Context,
        device: &wgpu::Device,
        source: evaluation::SuppliedImage,
        endpoint: evaluation::Value,
        destination: &wgpu::Texture,
    ) -> Result<Self, Error> {
        context.require_device(device)?;
        let layout = source.layout();
        if layout.format != evaluation::ImageFormat::Rgba32Float {
            return Err(Error::InvalidParameter("invalid rendered source storage"));
        }
        let region = layout.region;
        let request = context.request(
            endpoint.clone(),
            evaluation::Region {
                bounds: region,
                grid: layout.domain.grid,
            },
            evaluation::Tiling {
                tile: [region.width, region.height],
            },
        )?;
        let recorded = context.record_to_texture(
            request,
            context.materializations(),
            vec![source.clone()],
            destination,
        )?;
        Ok(Self {
            source,
            endpoint,
            recorded,
        })
    }

    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), Error> {
        self.recorded
            .encode_work(device, encoder, &[self.source.pixels()])?;
        Ok(())
    }

    pub(crate) fn byte_len(&self) -> u64 {
        self.recorded.byte_len()
    }

    pub(crate) fn endpoint(&self) -> &evaluation::Value {
        &self.endpoint
    }
}
