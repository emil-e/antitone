//! Shared photographic shader source for graph operations and device-free validation.

/// The exact module text the pipelines compile, exposed for device-free validation.
pub(crate) fn module_shader() -> String {
    [
        kernel_shader(),
        include_str!("../../shaders/footprint.wgsl").to_owned(),
        include_str!("../../shaders/repairs.wgsl").to_owned(),
    ]
    .join("\n")
}

/// One implementation serves the viewport, export and analysis: the near-base ratio response
/// and the f32 colour operators.
pub(crate) fn kernel_shader() -> String {
    [
        pixel_shader(),
        include_str!("../../shaders/processing_dispatch.wgsl").to_owned(),
    ]
    .join("\n")
}

pub(crate) fn pixel_shader() -> String {
    let common = [
        include_str!("../../shaders/camera_conversion.wgsl"),
        include_str!("../../shaders/flat_field.wgsl"),
        include_str!("../../shaders/film_response.wgsl"),
        include_str!("../../shaders/film_inversion_export.wgsl"),
        include_str!("../../shaders/photographic.wgsl"),
        include_str!("../../shaders/adjustment_layers.wgsl"),
        include_str!("../../shaders/photographic_colour.wgsl"),
        include_str!("../../shaders/photographic_export.wgsl"),
    ]
    .join("\n");
    [
        common.as_str(),
        include_str!("../../shaders/processing.wgsl"),
    ]
    .join("\n")
}
