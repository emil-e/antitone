pub use crate::gpu::execution::regional::Demand as ViewRequest;
use crate::{
    Error,
    gpu::{GpuFlatField, ProcessingInput},
    rendering::OutputTarget,
    scan::Scan,
    viewer::GpuImage,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

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
    let input = parameters.into();
    let cancelled = || generation.load(Ordering::Relaxed) != expected;
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
    request.validate()?;
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
        request,
        &output.linear,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    output.set_look_check(completed.look_check);
    Ok(Some(Arc::new(output)))
}
