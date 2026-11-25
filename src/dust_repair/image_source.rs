//! Original, unrepaired neutral image queries for Dust donor search.

use crate::{
    Error,
    dust_repair::{Ap1, BaseImage},
    geometry::SourceRegion,
    scan::Scan,
};
use std::sync::Arc;
use {crate::evaluation, crate::photographic::stages::flat_field::GpuFlatField};

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(crate) struct Source<'a> {
    context: &'a evaluation::Context,
    queue: &'a crate::gpu::submission::Queue,
    image: evaluation::Value,
    budget: evaluation::Tiling,
}

impl<'a> Source<'a> {
    pub(crate) fn from_recipe(
        field: Option<Arc<GpuFlatField>>,
        context: &'a evaluation::Context,
        gpu: &'a crate::gpu::submission::Queue,
        scan: &Scan,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<Self, Error> {
        recipe.neutral.require_scan(scan)?;
        let unrepaired = crate::photographic::graph::composition::original_unrepaired(
            crate::acquisition::camera::camera_source(scan)?,
            field,
            recipe,
        )?;
        Self::from_image(context, gpu, unrepaired)
    }

    pub(crate) fn from_image(
        context: &'a evaluation::Context,
        gpu: &'a crate::gpu::submission::Queue,
        image: evaluation::Value,
    ) -> Result<Self, Error> {
        Ok(Self {
            context,
            queue: gpu,
            image,
            budget: evaluation::Tiling { tile: [512, 128] },
        })
    }

    pub(crate) fn repair_succeeded(
        &self,
        repair: &crate::dust_repair::Repair,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<bool>, Error> {
        let healed =
            crate::photographic::stages::healing::operation::healing_image(&self.image, repair)?;
        let point = repair
            .support
            .selected_points()
            .next()
            .expect("validated repair support is nonempty");
        let region = evaluation::CellRegion::new(point.map(|v| v as i32), [1, 1]);
        let query = self.context.request(
            healed.clone(),
            evaluation::Region {
                bounds: region,
                grid: healed.domain().grid,
            },
            self.budget,
        )?;
        let mut succeeded = false;
        let result = self
            .context
            .read(self.queue, &query, cancelled, |_, pixels| {
                succeeded = pixels[0][3] >= 0.;
                Ok(())
            })?;
        Ok(result.map(|_| succeeded))
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

        let result = self
            .context
            .read(self.queue, &evaluation, cancelled, |block, values| {
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
            })?;

        Ok(result.map(|_| pixels))
    }
}
