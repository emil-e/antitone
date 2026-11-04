//! Raster adapter for shared image-node composition and declared GPU work.

use super::{evaluation, raster::Raster};
use crate::Error;
#[cfg(test)]
use crate::pixel_pipeline::PixelPipeline;
use std::sync::Arc;

pub(crate) struct Prepared {
    pub raster: Raster,
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
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                raster.size(),
            )),
            grid: evaluation::Grid::from_raster(raster),
            meaning: evaluation::Meaning::Graded,
        };
        let root = pipeline.compose(evaluation::raster_input(domain))?;
        if pipeline.is_empty() {
            return Ok(Self {
                raster,
                execution: None,
            });
        }
        Self::from_composition(device, raster, root, context)
    }

    pub fn from_composition(
        device: &wgpu::Device,
        raster: Raster,
        root: Arc<evaluation::ImageNode>,
        context: evaluation::Context,
    ) -> Result<Self, Error> {
        let mut raster = raster;
        raster.dimensions[2] = root.domain().grid.coverage;
        let expected = evaluation::Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                raster.size(),
            )),
            grid: evaluation::Grid::from_raster(raster),
            meaning: root.domain().meaning,
        };
        if root.domain().bounds != expected.bounds || !root.domain().same_grid(expected) {
            return Err(Error::InvalidParameter(
                "recorded composition differs from raster",
            ));
        }
        Self::record_composition(
            device,
            raster,
            root,
            expected.finite()?,
            context,
            device.limits().max_buffer_size.saturating_mul(4),
            Vec::new(),
        )
    }

    /// Record an explicitly composed grid transition into its destination-sized sink.
    pub(crate) fn record_composition(
        device: &wgpu::Device,
        raster: Raster,
        root: Arc<evaluation::ImageNode>,
        region: evaluation::CellRegion,
        context: evaluation::Context,
        scratch_bytes: u64,
        supplied: Vec<evaluation::SuppliedImage>,
    ) -> Result<Self, Error> {
        context.require_device(device)?;
        let output = root.domain();
        let evaluation = context.request(
            root.clone(),
            evaluation::Region {
                bounds: region,
                grid: output.grid,
            },
            evaluation::Budget {
                tile: [region.width, region.height],
                scratch_bytes,
            },
        )?;
        let execution = context.record(evaluation, context.materializations(), supplied)?;
        Ok(Self {
            raster,
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

    pub fn submitted(&self) {
        if let Some(execution) = &self.execution {
            execution.submitted();
        }
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn scratch_bytes(&self) -> u64 {
        self.execution
            .as_ref()
            .map_or(0, |work| work.scratch_bytes())
    }

    pub fn byte_len(&self) -> u64 {
        self.execution.as_ref().map_or(0, |p| p.byte_len())
    }
}
