//! Render a prepared photographic recipe for file export.
use super::*;
use crate::photographic::stages::flat_field::GpuFlatField;
use crate::photographic::stages::{display_rendering as drt, look::Looks};
use crate::{dust_repair::Execution, image_recipe::ImageRecipe, scan::DevelopedSource};

pub(crate) struct Job<'a> {
    pub source: &'a Arc<DevelopedSource>,
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

    let recipe = job.recipe;
    recipe.neutral.require_source(job.source)?;
    if !recipe.neutral.input.is_negative() && recipe.neutral.reference.is_none() {
        return Err(Error::InvalidParameter(
            "export requires resolved Digital starting exposure",
        ));
    }
    let image = crate::photographic::graph::composition::processed_originals(
        &context.device().limits(),
        crate::acquisition::camera::camera_source(job.source)?,
        job.field,
        recipe,
    )?;
    let definition = match job.render.display {
        Some(Display {
            looks,
            targets,
            target,
        }) => crate::photographic::graph::display_output(
            image,
            &recipe.luts,
            looks,
            targets.get(target),
        )?,
        None => image,
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
