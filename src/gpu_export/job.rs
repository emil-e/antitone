//! Resolve captured photographic intent into a file export calculation.
use super::*;
use crate::gpu::{drt, flat::GpuFlatField, lut3d::Looks};
use crate::{dust_repair::Execution, image_recipe::ImageRecipe, scan::Scan};

pub(crate) struct Job<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub recipe: &'a ImageRecipe,
    pub render: Render<'a>,
}

pub(crate) struct Render<'a> {
    plan: Plan,
    display: Option<Display<'a>>,
}

struct Display<'a> {
    looks: &'a Looks,
    targets: Arc<drt::Targets>,
    target: crate::rendering::OutputTarget,
}

impl<'a> Render<'a> {
    pub(crate) fn resolve<E>(
        plan: Plan,
        display: impl FnOnce() -> Result<(&'a Looks, Arc<drt::Targets>), E>,
    ) -> Result<Self, E> {
        let display = match plan.target() {
            RenderTarget::Display(target) => {
                let (looks, targets) = display()?;
                Some(Display {
                    looks,
                    targets,
                    target,
                })
            }
            RenderTarget::SceneLinearAp0 => None,
        };
        Ok(Self { plan, display })
    }
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
    let pipeline = crate::photographic::graph::RecipeGraph::originals(
        crate::gpu::source::camera_source(&scan)?,
        job.field,
        &recipe,
    )?;
    let limits = context.device().limits();
    let definition = match job.render.display {
        Some(Display {
            looks,
            targets,
            target,
        }) => pipeline.display(&limits, looks, targets.get(target))?,
        None => pipeline.processed(&limits)?,
    };
    super::area::write_calculation(
        context,
        gpu,
        Request {
            definition,
            composition: recipe.composition,
            plan: job.render.plan,
        },
        destination,
        check,
    )
    .map(|_| ())
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
