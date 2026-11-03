use crate::Error;
pub use crate::gpu::execution::regional::Demand as ViewRequest;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::input::ProcessingInput;
use crate::rendering::OutputTarget;
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
    parameters: impl Into<ProcessingInput>,
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

pub(crate) fn refine_cancellable(
    renderer: &Arc<crate::renderer::GpuRenderer>,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &crate::platform::SubmissionGate),
    (scan, field): (&Scan, Option<Arc<GpuFlatField>>),
    parameters: impl Into<ProcessingInput>,
    (target, view): (OutputTarget, ViewRequest),
    cancelled: impl Fn() -> bool,
) -> Result<Option<Arc<GpuImage>>, Error> {
    let input = parameters.into();
    if cancelled() {
        return Ok(None);
    }
    let request = crate::gpu::execution::regional::Request {
        scan,
        field,
        input: &input,
        target,
        demand: view,
    };
    let root = request.definition(device, renderer)?;
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
        !input.luts().is_empty(),
        &output.linear,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    output.set_look_check(completed.look_check);
    Ok(Some(Arc::new(output)))
}
