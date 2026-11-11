//! Original, unrepaired neutral image queries for Dust donor search.

use crate::{
    Error,
    dust_repair::{Ap1, BaseImage},
    geometry::SourceRegion,
    renderer::GpuRenderer,
    scan::Scan,
};
use std::sync::Arc;
use {
    super::evaluation,
    crate::gpu::{flat::GpuFlatField, submission::SubmissionGate},
};

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(crate) struct Source<'a> {
    context: &'a evaluation::Context,
    queue: &'a wgpu::Queue,
    submissions: &'a SubmissionGate,
    image: evaluation::Value,
    budget: evaluation::Budget,
}

impl<'a> Source<'a> {
    pub(crate) fn from_recipe(
        device: &wgpu::Device,
        renderer: &'a GpuRenderer,
        gpu: (&'a wgpu::Queue, &'a SubmissionGate),
        scan: &Scan,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<Self, Error> {
        recipe.neutral.require_scan(scan)?;
        let field = recipe
            .neutral
            .correction
            .clone()
            .map(|field| GpuFlatField::new(device, field).map(Arc::new))
            .transpose()?;
        let graph = crate::gpu::photograph::RecipeGraph::originals(
            evaluation::camera_source(scan)?,
            field,
            recipe,
        )?;
        Self::from_image(renderer, gpu, graph.interpreted()?.unrepaired)
    }

    pub(crate) fn from_image(
        renderer: &'a GpuRenderer,
        gpu: (&'a wgpu::Queue, &'a SubmissionGate),
        image: evaluation::Value,
    ) -> Result<Self, Error> {
        Ok(Self {
            context: renderer.evaluation(),
            queue: gpu.0,
            submissions: gpu.1,
            image,
            budget: evaluation::Budget {
                tile: [512, 128],
                scratch_bytes: 32 << 20,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn image(&self) -> &evaluation::Value {
        &self.image
    }
}

impl BaseImage for Source<'_> {
    fn read(
        &mut self,
        region: SourceRegion,
        needed: Option<&[bool]>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Ap1>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        region.validate()?;
        let cells = usize::try_from(u64::from(region.width) * u64::from(region.height))
            .map_err(|_| Error::InvalidParameter("repair input region is too large"))?;
        if needed.is_some_and(|mask| mask.len() != cells) {
            return Err(Error::InvalidParameter(
                "repair input selection differs from its region",
            ));
        }
        let evaluation = self.context.request(
            self.image.clone(),
            evaluation::Region {
                bounds: region.try_into()?,
                grid: self.image.domain().grid,
            },
            self.budget,
        )?;
        let mut pixels = vec![[0.; 3]; cells];

        let result = self.context.read(
            self.queue,
            self.submissions,
            &evaluation,
            cancelled,
            |block, values| {
                let block = block.source()?;
                for (index, pixel) in values.iter().enumerate() {
                    let x = block.x + index as u32 % block.width - region.x;
                    let y = block.y + index as u32 / block.width - region.y;
                    let destination =
                        (u64::from(y) * u64::from(region.width) + u64::from(x)) as usize;
                    if needed.is_none_or(|mask| mask[destination]) {
                        let rgb = [pixel[0], pixel[1], pixel[2]];
                        if !rgb.iter().all(|value| value.is_finite()) {
                            return Err(Error::NumericalOverflow);
                        }
                        pixels[destination] = rgb;
                    }
                }
                Ok(())
            },
        )?;

        Ok(result.map(|_| pixels))
    }
}
