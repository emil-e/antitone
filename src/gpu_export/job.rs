//! Immutable, source-validated preparation for the shared TIFF/EXR engine.
use super::*;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::submission::SubmissionGate;
use crate::{dust_repair::Execution, image_recipe::ImageRecipe, scan::Scan};

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

    let (scan, recipe) = crate::photo_preparation::native_image(job.scan, job.recipe)?;
    let photograph = crate::gpu::photograph::PhotographGraph::from_recipe(
        &device.limits(),
        crate::gpu::evaluation::camera_source(&scan)?,
        job.field,
        &recipe,
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
