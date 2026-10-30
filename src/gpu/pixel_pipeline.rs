//! Raster adapter for shared image-node composition and declared GPU work.

use super::{evaluation, raster::Raster};
use crate::Error;
#[cfg(test)]
use crate::pixel_pipeline::PixelPipeline;
use std::sync::Arc;

pub(crate) struct Kernels {
    #[cfg(all(test, feature = "editor"))]
    pub mtf: super::image_mtf::Kernels,
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
        Self::record_composition(
            device,
            raster,
            root,
            previous,
            context,
            device.limits().max_buffer_size.saturating_mul(4),
        )
    }

    /// Record an explicitly composed grid transition into its destination-sized sink.
    pub(crate) fn record_composition(
        device: &wgpu::Device,
        raster: Raster,
        root: Arc<evaluation::ImageNode>,
        previous: Option<&Self>,
        context: evaluation::Context,
        scratch_bytes: u64,
    ) -> Result<Self, Error> {
        context.require_device(device)?;
        let output = root.domain;
        let evaluation = context.request(
            root,
            evaluation::Region {
                bounds: output.bounds,
                grid: output.grid,
            },
            evaluation::Budget {
                tile: [output.bounds.width, output.bounds.height],
                scratch_bytes,
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

impl Kernels {
    pub async fn new(_device: &wgpu::Device) -> Result<Self, Error> {
        Ok(Self {
            #[cfg(all(test, feature = "editor"))]
            mtf: super::image_mtf::Kernels::new(_device).await?,
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
}
