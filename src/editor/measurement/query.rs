//! Worker-side original-grid inputs and weighted statistics for interactive measurements.

use crate::Error;
use crate::coverage::Coverage;
use crate::evaluation;
use crate::evaluation::Grid;
use crate::evaluation::Region;
use crate::evaluation::Tiling;
use crate::evaluation::Value;
use crate::photographic::graph::camera_light;
use crate::photographic::stages::flat_field::GpuFlatField;
use crate::scan::Scan;
use glam::DVec3;
use std::sync::Arc;

pub(in crate::editor) const FORWARD_RESIDUAL_FLOOR: f64 = 16. * f32::EPSILON as f64;

#[cfg(test)]
mod tests;

pub(in crate::editor) struct Samples<'a> {
    context: &'a evaluation::Context,
    queue: &'a crate::gpu::submission::Queue,
    scan: &'a Scan,
    pub(in crate::editor) source: Value,
    field: Option<Arc<GpuFlatField>>,
    coverage: Coverage,
}

impl<'a> Samples<'a> {
    pub(in crate::editor) fn new(
        (context, queue): super::Gpu<'a>,
        scan: &'a mut Scan,
        field: Option<Arc<GpuFlatField>>,
        coverage: Coverage,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }

        coverage.bounds(scan.area)?;

        let source = crate::acquisition::camera::camera_source(scan)?;

        Ok(Self {
            context,
            queue,
            scan,
            source,
            field,
            coverage,
        })
    }

    pub(in crate::editor) fn read(
        &self,
        image: Value,
        cancelled: &dyn Fn() -> bool,
        mut consume: impl FnMut([u32; 2], [f32; 4], f64) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let evaluation = self.context.request(
            image,
            Region {
                bounds: self.coverage.bounds(self.scan.area)?.try_into()?,
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
            Tiling { tile: [512, 128] },
        )?;

        let result = self
            .context
            .read(self.queue, &evaluation, cancelled, |region, pixels| {
                let region = region.source()?;
                for (index, pixel) in pixels.iter().enumerate() {
                    let point = [
                        region.x + index as u32 % region.width,
                        region.y + index as u32 / region.width,
                    ];
                    let area = self.coverage.weight(point);
                    if area > 0. {
                        consume(point, *pixel, area)?;
                    }
                }
                Ok(())
            })?;
        result.ok_or(Error::Cancelled)?;

        Ok(())
    }

    pub(in crate::editor) fn camera_light(&self) -> Result<Value, Error> {
        camera_light(self.source.clone(), self.field.clone())
    }

    pub(in crate::editor) fn require_precision(
        &self,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Error> {
        // Match the precision bound packed into the photographic program. Comparing readback
        // pixels with an unrounded f64 floor can reject the representable boundary itself.
        let floor = self.scan.effective_floor() as f32;
        self.read(self.camera_light()?, cancelled, |_, pixel, _| {
            if pixel[..3]
                .iter()
                .any(|value| !value.is_finite() || *value < floor)
            {
                return Err(Error::UnusableSample(
                    "patch contains nonfinite or floored scan inputs",
                ));
            }
            Ok(())
        })
    }

    pub(in crate::editor) fn roll(
        &self,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<Value, Error> {
        recipe.neutral.require_scan(self.scan)?;
        crate::photographic::graph::composition::measured_roll(
            self.source.clone(),
            self.field.clone(),
            recipe,
        )
    }

    pub(in crate::editor) fn graded(
        &self,
        limits: &wgpu::Limits,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<Value, Error> {
        recipe.neutral.require_scan(self.scan)?;
        crate::photographic::graph::composition::measured_grade(
            limits,
            self.source.clone(),
            self.field.clone(),
            recipe,
        )
    }

    pub(in crate::editor) fn mean(
        &self,
        image: Value,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DVec3, Error> {
        let mut sum = DVec3::ZERO;
        let mut total = 0.;

        self.read(image, cancelled, |_, pixel, area| {
            let rgb = DVec3::new(
                f64::from(pixel[0]),
                f64::from(pixel[1]),
                f64::from(pixel[2]),
            );
            if !rgb.is_finite() || !pixel[3].is_finite() || pixel[3] < 0. {
                return Err(Error::NumericalOverflow);
            }

            if pixel[3] as u32 & 7 != 0 {
                return Err(Error::UnusableSample(
                    "measurement patch contains inputs below the calibration floor",
                ));
            }

            sum += rgb * area;
            total += area;
            Ok(())
        })?;

        if total == 0. {
            return Err(Error::UnusableSample("empty patch"));
        }

        let mean = sum / total;
        if !mean.is_finite() {
            return Err(Error::NumericalOverflow);
        }

        Ok(mean)
    }
}
