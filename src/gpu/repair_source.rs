//! Shared photographic image inputs for CPU donor search and healing.

use super::{GpuFlatField, SubmissionGate, evaluation};
use crate::{
    Error,
    dust_repair::{Ap1, BaseImage, EvaluatedRepair, Execution},
    geometry::SourceRegion,
    processing::InputSettings,
    renderer::GpuRenderer,
    scan::Scan,
};
use std::sync::Arc;

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(crate) struct Source<'a> {
    context: &'a evaluation::Context,
    queue: &'a wgpu::Queue,
    submissions: &'a SubmissionGate,
    image: Arc<evaluation::ImageNode>,
    budget: evaluation::Budget,
}

impl<'a> Source<'a> {
    pub(crate) fn new(
        device: &wgpu::Device,
        renderer: &'a GpuRenderer,
        gpu: (&'a wgpu::Queue, &'a SubmissionGate),
        scan: &Scan,
        input: InputSettings,
    ) -> Result<Self, Error> {
        let program = crate::gpu::GpuInputProgram::for_scan(scan, input)?;
        let field = scan
            .flat_field()
            .cloned()
            .map(|field| GpuFlatField::new(device, field).map(Arc::new))
            .transpose()?;
        let readout = evaluation::Photograph::neutral_readout(
            device,
            renderer,
            evaluation::camera_source(scan)?,
            field,
            program,
        )?;

        Ok(Self {
            context: renderer.evaluation(),
            queue: gpu.0,
            submissions: gpu.1,
            image: readout.decoded(),
            budget: evaluation::Budget {
                tile: [512, 128],
                scratch_bytes: 32 << 20,
            },
        })
    }
}

pub(crate) fn resolve_plan(
    device: &wgpu::Device,
    renderer: &GpuRenderer,
    gpu: (&wgpu::Queue, &SubmissionGate),
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    plan: &crate::acquisition::Plan,
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<Arc<EvaluatedRepair>>>, Error> {
    // A valid folded original endpoint must not acquire a neutral-image requirement when
    // no contributing repair requests that image. Capture it only at the first such request.
    let mut source = None;
    crate::repair_source::resolve_plan_using(scan, recipe, plan, execution, |repair, execution| {
        if (execution.cancelled)() {
            return Ok(None);
        }
        if source.is_none() {
            source = Some(Source::new(
                device,
                renderer,
                gpu,
                scan,
                recipe.neutral.processing_input()?,
            )?);
        }
        crate::dust_repair::evaluate_image(repair, scan.area, source.as_mut().unwrap(), execution)
    })
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
                bounds: region,
                grid: self.image.domain.grid,
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
