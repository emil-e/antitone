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
fn shared_export_assembles_guarded_response_and_common_colour_without_changing_stage_routing() {
    let text = shader();
    assert_eq!(text.matches("fn invert_film(").count(), 1);
    assert!(text.contains("precise_sub(reference, sample).h / sample.h"));
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
    let control = ProcessingKernel::shader();
    assert!(control.contains(include_str!("shaders/photographic_precise.wgsl")));
    assert!(!text.contains(include_str!("shaders/photographic_precise.wgsl")));
    assert!(!control.contains("response_log1p"));
    assert!(control.contains(include_str!("shaders/film_inversion_precise.wgsl")));
}
