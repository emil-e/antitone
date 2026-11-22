//! Immutable, source-validated preparation for the shared TIFF/EXR engine.
use super::*;
use crate::gpu::{drt, flat::GpuFlatField, lut3d::Looks};
use crate::{dust_repair::Execution, image_recipe::ImageRecipe, scan::Scan};

pub(crate) struct Job<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub recipe: &'a ImageRecipe,
    pub output: Output<'a>,
}

pub(crate) enum Output<'a> {
    Tiff {
        looks: &'a Looks,
        target: &'a drt::Target,
    },
    Exr(ExrPrecision),
}

pub(crate) fn write(
    context: &evaluation::Context,
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
        crate::gpu::source::camera_source(&scan)?,
        job.field,
        &recipe,
    )?;
    let limits = context.device().limits();
    let (definition, format) = match job.output {
        Output::Tiff { looks, target } => (
            pipeline.display(&limits, looks, target)?,
            Format::Tiff(target.output_target()),
        ),
        Output::Exr(precision) => (pipeline.processed(&limits)?, Format::Exr(precision)),
    };
    super::area::write_calculation(
        context,
        gpu,
        Request {
            definition,
            composition: recipe.composition,
            format,
        },
        destination,
        check,
    )
    .map(|_| ())
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
