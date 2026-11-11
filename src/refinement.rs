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
    renderer: &Arc<crate::renderer::GpuRenderer>,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &crate::platform::SubmissionGate),
    (scan, field): (&Scan, Option<Arc<GpuFlatField>>),
    parameters: impl Into<AuthoredPhotograph>,
    target: OutputTarget,
    (generation, expected): (&AtomicU64, u64),
    view: ViewRequest,
) -> Result<Option<Arc<GpuImage>>, Error> {
    refine_cancellable(
        renderer,
        device,
        (queue, submissions),
        (scan, field),
        parameters,
        (target, view),
        || generation.load(Ordering::Relaxed) != expected,
    )
}

#[cfg(test)]
pub(crate) fn refine_cancellable(
    renderer: &Arc<crate::renderer::GpuRenderer>,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &crate::platform::SubmissionGate),
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
    let root = crate::gpu::evaluation::original_chain(
        device,
        renderer,
        scan,
        field,
        &input,
        crate::rendering::OutputReference::DisplayReferred(target),
        &input.pixel_pipeline(),
    )?;
    refine_definition(
        renderer,
        device,
        (queue, submissions),
        root,
        (target, view),
        !input.luts().is_empty(),
        cancelled,
    )
}

pub(crate) fn refine_definition(
    renderer: &Arc<crate::renderer::GpuRenderer>,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &crate::platform::SubmissionGate),
    original: crate::gpu::evaluation::Value,
    (target, view): (OutputTarget, ViewRequest),
    check_look: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Arc<GpuImage>>, Error> {
    if cancelled() {
        return Ok(None);
    }
    let root = crate::gpu::execution::regional::definition(original, view)?;
    let output = pollster::block_on(GpuImage::new_rendered_shared(
        renderer,
        device,
        view.output[0],
        view.output[1],
        target,
    ))?;
    let Some(completed) = crate::gpu::execution::regional::execute(
        renderer,
        device,
        (queue, submissions),
        root,
        check_look,
        &output.linear,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    output.set_look_check(completed.look_check);
    Ok(Some(Arc::new(output)))
}
