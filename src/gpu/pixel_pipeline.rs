//! Raster adapter for declared GPU work over an explicit image value.

use super::{evaluation, raster::Raster};
use crate::Error;
#[cfg(test)]
use crate::pixel_pipeline::PixelPipeline;

pub(crate) struct Prepared {
    execution: Option<evaluation::Recorded>,
}

impl Prepared {
    #[cfg(test)]
    pub fn new(
        device: &wgpu::Device,
        pipeline: &PixelPipeline,
        raster: Raster,
    ) -> Result<Self, Error> {
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
        pipeline: &PixelPipeline,
        raster: Raster,
        context: evaluation::Context,
    ) -> Result<Self, Error> {
        let domain = evaluation::Domain {
            bounds: Some(evaluation::CellRegion::new([0, 0], raster.size())),
            grid: evaluation::Grid::from_raster(raster),
            meaning: evaluation::Meaning::Graded,
        };
        let root = pipeline.compose(evaluation::raster_input(domain))?;
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
            meaning: root.domain().meaning,
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
            evaluation::Budget {
                tile: [region.width, region.height],
                scratch_bytes: device.limits().max_buffer_size.saturating_mul(4),
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
