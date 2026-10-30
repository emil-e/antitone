//! Immutable, source-validated preparation for the shared TIFF/EXR engine.
use super::*;
use crate::{dust_repair::Execution, image_recipe::ImageRecipe};

pub(crate) struct Job<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub recipe: &'a ImageRecipe,
    pub format: Format,
}

pub(crate) fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    job: Job<'_>,
    destination: (&Path, Overwrite),
    execution: &mut Execution<'_>,
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    let cancelled = execution.cancelled;
    let mut check = || {
        check()?;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    };
    check()?;

    let reference = job
        .recipe
        .neutral
        .reference
        .clone()
        .or_else(|| job.scan.reference_exposure.clone())
        .map_or_else(std::sync::OnceLock::new, std::sync::OnceLock::from);
    let prepared = crate::photo_preparation::native(
        None,
        &reference,
        job.recipe.neutral.input,
        job.scan.source(),
    )?;
    let mut scan = prepared.scan();
    scan.set_flat_field(job.scan.flat_field().cloned())?;
    job.recipe.neutral.require_scan(&scan)?;
    let input = job.recipe.neutral.input;
    let stages = job.recipe.pixel_pipeline()?;
    let plan = crate::acquisition::Plan::new(crate::acquisition::Request {
        output: FrameRaster::full(job.recipe.composition)?,
        stages: &stages,
        sampling: crate::acquisition::Sampling::Originals {
            reconstruction: crate::acquisition::SourceSupport([0; 2]),
        },
    })?;
    let repairs = timed!(
        SourcePrepare,
        crate::gpu::repair_source::resolve_plan(
            device, renderer, gpu, &scan, job.recipe, &plan, execution
        )
    )?
    .ok_or(Error::Cancelled)?;
    check()?;
    let request = Request {
        scan: &scan,
        field: job.field,
        input,
        roll: job.recipe.roll,
        grade: job.recipe.grade.clone(),
        repairs: &repairs,
        composition: job.recipe.composition,
        format: job.format,
        look: job.recipe.look.clone(),
        film_emulation: job.recipe.film_emulation,
        film_seed: job.recipe.neutral.source.0,
    };
    super::area::write_composition(renderer, device, gpu, (request, stages), destination, check)
        .map(|_| ())
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
