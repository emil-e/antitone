//! Immutable, source-validated preparation for the shared TIFF/EXR engine.
use super::*;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::submission::SubmissionGate;
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
    // Preparation may resolve an unrecorded positive exposure baseline; freeze it with this job.
    let mut recipe = job.recipe.clone();
    recipe.neutral.reference = scan.reference_exposure.clone();
    let photograph = crate::gpu::photograph::PhotographGraph::from_recipe(
        &device.limits(),
        crate::gpu::evaluation::camera_source(&scan)?,
        job.field,
        &recipe,
        &crate::gpu::repairs::Snapshot::logical(repairs)?,
    )?;
    let output = match job.format {
        Format::Tiff(target) => crate::rendering::OutputReference::DisplayReferred(target),
        Format::Exr(_) => crate::rendering::OutputReference::SceneReferred,
    };
    let definition =
        crate::gpu::evaluation::Reference::capture(device, renderer, output, &recipe.luts)?
            .compose(photograph.processed, renderer)?;
    super::area::write_calculation(
        renderer,
        device,
        gpu,
        definition,
        recipe.composition,
        job.format,
        destination,
        check,
    )
    .map(|_| ())
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
