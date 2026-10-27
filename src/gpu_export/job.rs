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
    job.recipe.neutral.require_scan(job.scan)?;
    let prepared = crate::source::SourceCache::new(0)
        .prepare_source(job.recipe.neutral.input, job.scan.source())?;
    let mut scan = Scan::from_source(prepared.source);
    scan.set_flat_field(job.scan.flat_field().cloned())?;
    let input = job.recipe.neutral.input;
    let repairs = timed!(
        SourcePrepare,
        crate::repair_source::resolve_output_with(
            &scan,
            job.recipe,
            job.recipe.pixel_pipeline()?.support([1., 1.])?,
            execution
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
    super::write_checked(
        renderer,
        device,
        gpu,
        request,
        destination.0,
        destination.1,
        check,
    )
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
