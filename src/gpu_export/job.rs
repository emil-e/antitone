//! Immutable, source-validated preparation for the shared TIFF/EXR engine.
use super::*;
use crate::{dust_repair::Execution, image_recipe::ImageRecipe, scan::Scan};
use crate::{gpu::flat::GpuFlatField, renderer::GpuRenderer};

pub(crate) struct Job<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub recipe: &'a ImageRecipe,
    pub format: Format,
}

pub(crate) fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: &crate::gpu::submission::Queue,
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
    let pipeline = crate::gpu::photograph::RecipeGraph::originals(
        crate::gpu::evaluation::camera_source(&scan)?,
        job.field,
        &recipe,
    )?;
    let limits = device.limits();
    let definition = match job.format {
        Format::Tiff(target) => {
            pipeline.display(&limits, &renderer.looks, renderer.drt.get(target))
        }
        Format::Exr(_) => pipeline.processed(&limits),
    }?;
    super::area::write_calculation(
        renderer.evaluation(),
        gpu,
        Request {
            definition,
            composition: recipe.composition,
            format: job.format,
        },
        destination,
        check,
    )
    .map(|_| ())
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
