//! Diagnostic arithmetic controls for the shared export kernel, independent of reconstruction.
use super::*;

fn shader() -> String {
    ProcessingKernel::export_shader()
}

// Operator/reference diagnostics need to select arithmetic independently of reconstruction.
// Normal exports use the existing renderer's lazily cached export pipeline instead.
pub(crate) async fn renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gate: SubmissionGate,
) -> Result<Arc<crate::renderer::GpuRenderer>, Error> {
    let mut renderer =
        crate::renderer::GpuRenderer::new_with_submissions(device, queue, gate).await?;
    Arc::get_mut(&mut renderer)
        .expect("new diagnostic renderer uniquely owned")
        .kernel = ProcessingKernel::new_export(device).await?;
    Ok(renderer)
}

#[path = "gpu_export_density_condition_tests.rs"]
mod density_condition;
#[path = "gpu_export_precision_margin_tests.rs"]
mod precision_margins;
#[path = "gpu_export_arithmetic_research_tests.rs"]
pub(crate) mod tests;

#[test]
fn one_photographic_composition_serves_the_viewport_and_export() {
    let text = kernel_shader();
    assert_eq!(text.matches("fn invert_film(").count(), 1);
    // The response keeps the near-base ratio density, which is the one place the model needs it.
    assert!(text.contains("/ sample.h"));
    for source in [
        include_str!("shaders/camera_conversion.wgsl"),
        include_str!("shaders/flat_field.wgsl"),
        include_str!("shaders/photographic.wgsl"),
        include_str!("shaders/local_adjustments.wgsl"),
        include_str!("shaders/photographic_colour.wgsl"),
        include_str!("shaders/photographic_export.wgsl"),
        include_str!("shaders/processing.wgsl"),
    ] {
        assert!(text.contains(source));
    }
    // The viewport kernel and the export kernel are the same implementation.
    assert_eq!(text, ProcessingKernel::kernel_shader());
    assert_eq!(text, ProcessingKernel::export_shader());
}
