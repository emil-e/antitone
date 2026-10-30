//! Worker-side original-grid inputs and weighted statistics for interactive measurements.

use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    coverage::Coverage,
    gpu::{
        GpuFlatField, Grading, ProcessingInput,
        evaluation::{self, Budget, Grid, ImageNode, Photograph, Region},
    },
    processing::{InputSettings, NegativeInput, RollSettings},
    renderer::GpuRenderer,
    scan::Scan,
};
use glam::DVec3;
use std::sync::Arc;

pub(in crate::editor) const FORWARD_RESIDUAL_FLOOR: f64 = 16. * f32::EPSILON as f64;

#[cfg(test)]
mod tests;

pub(in crate::editor) struct Samples<'a> {
    services: &'a crate::editor::source::Services,
    scan: &'a Scan,
    renderer: Arc<GpuRenderer>,
    pub(in crate::editor) source: Arc<ImageNode>,
    field: Option<Arc<GpuFlatField>>,
    coverage: Coverage,
}

impl<'a> Samples<'a> {
    pub(in crate::editor) fn new(
        services: &'a crate::editor::source::Services,
        scan: &'a mut Scan,
        coverage: Coverage,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }

        scan.checked_coverage(coverage)?;

        let renderer = services.renderers.get(&services.device, &services.queue)?;
        let source = evaluation::camera_source(scan)?;
        let field = scan
            .flat_field()
            .cloned()
            .map(|field| GpuFlatField::new(&services.device, field).map(Arc::new))
            .transpose()?;

        Ok(Self {
            services,
            scan,
            renderer,
            source,
            field,
            coverage,
        })
    }

    pub(in crate::editor) fn read(
        &self,
        image: Arc<ImageNode>,
        cancelled: &dyn Fn() -> bool,
        mut consume: impl FnMut([u32; 2], [f32; 4], f64) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let evaluation = self.renderer.evaluation().request(
            image,
            Region {
                bounds: self.coverage.bounds(self.scan.area)?,
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
            Budget {
                tile: [512, 128],
                scratch_bytes: 32 << 20,
            },
        )?;

        let result = self.renderer.evaluation().read(
            &self.services.queue,
            self.services.renderers.submissions(),
            &evaluation,
            cancelled,
            |region, pixels| {
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
            },
        )?;
        result.ok_or(Error::Cancelled)?;

        Ok(())
    }

    pub(in crate::editor) fn camera_light(&self) -> Result<Arc<ImageNode>, Error> {
        Photograph::camera_light(
            &self.services.device,
            &self.renderer.camera,
            self.source.clone(),
            self.field.clone(),
        )
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

    pub(in crate::editor) fn camera_response(
        &self,
        negative: NegativeInput,
    ) -> Result<evaluation::CameraReadout, Error> {
        evaluation::CameraReadout::normalized(self.camera_light()?, negative)
    }

    pub(in crate::editor) fn photograph(
        &self,
        input: InputSettings,
        roll: RollSettings,
        layers: AdjustmentLayerSnapshot,
    ) -> Result<Photograph, Error> {
        let program = crate::gpu::GpuInputProgram::for_scan(self.scan, input)?;
        let matrix = self.scan.positive_matrix(&input)?;

        let grade = Arc::new(Grading::new(
            &self.services.device,
            roll,
            layers,
            self.scan.area,
        )?);
        let pipeline = Arc::new(ProcessingInput::compose(
            &self.services.device,
            program,
            grade,
            self.scan.area,
        )?);

        Photograph::new(
            &self.services.device,
            &self.renderer.camera,
            self.source.clone(),
            self.field.clone(),
            &ProcessingInput::from(pipeline),
            matrix,
        )
    }

    pub(in crate::editor) fn mean(
        &self,
        image: Arc<ImageNode>,
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
