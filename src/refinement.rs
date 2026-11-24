use crate::Error;
pub use crate::gpu::execution::regional::Demand as ViewRequest;
#[cfg(test)]
use crate::gpu::flat::GpuFlatField;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::rendering::OutputTarget;
#[cfg(test)]
use crate::scan::Scan;
use crate::viewer::GpuImage;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn refine(
    renderer: &Arc<crate::render_fixture::RenderFixture>,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    (scan, field): (&Scan, Option<Arc<GpuFlatField>>),
    parameters: impl Into<AuthoredPhotograph>,
    target: OutputTarget,
    (generation, expected): (&AtomicU64, u64),
    view: ViewRequest,
) -> Result<Option<Arc<GpuImage>>, Error> {
    refine_cancellable(
        renderer,
        device,
        queue,
        (scan, field),
        parameters,
        (target, view),
        || generation.load(Ordering::Relaxed) != expected,
    )
}

#[cfg(test)]
pub(crate) fn refine_cancellable(
    renderer: &Arc<crate::render_fixture::RenderFixture>,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    (scan, field): (&Scan, Option<Arc<GpuFlatField>>),
    parameters: impl Into<AuthoredPhotograph>,
    (target, view): (OutputTarget, ViewRequest),
    cancelled: impl Fn() -> bool,
) -> Result<Option<Arc<GpuImage>>, Error> {
    let input = parameters.into();
    if cancelled() {
        return Ok(None);
    }
    scan.validate_area(view.area)?;
    let root = crate::photographic::graph::authoring::original_chain(
        &device.limits(),
        scan,
        field,
        &input,
        &input.definition(),
        renderer.looks(),
        renderer.drt(target),
    )?;
    refine_definition(
        renderer.evaluation(),
        device,
        queue,
        root,
        (target, view),
        cancelled,
    )
}

pub(crate) fn refine_definition(
    evaluation: &crate::gpu::evaluation::Context,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    original: crate::gpu::evaluation::Value,
    (target, view): (OutputTarget, ViewRequest),
    cancelled: impl Fn() -> bool,
) -> Result<Option<Arc<GpuImage>>, Error> {
    if cancelled() {
        return Ok(None);
    }
    let root = crate::gpu::execution::regional::definition(original, view)?;
    let output = pollster::block_on(GpuImage::new_rendered(
        device,
        view.output[0],
        view.output[1],
        target,
    ))?;
    let Some(()) = crate::gpu::execution::regional::execute(
        evaluation,
        device,
        queue,
        root,
        &output.linear,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(Arc::new(output)))
}
