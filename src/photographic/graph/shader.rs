//! Shared photographic functions and their packed resource bindings.

pub(crate) fn functions() -> String {
    [
        include_str!("../../gpu/parameters.wgsl"),
        include_str!("../../gpu/source/bindings.wgsl"),
        include_str!("../../gpu/flat/bindings.wgsl"),
        include_str!("../../gpu/grading/bindings.wgsl"),
        include_str!("pixel.wgsl"),
        include_str!("../../gpu/working_range.wgsl"),
        include_str!("../../gpu/interpretation/camera_conversion.wgsl"),
        include_str!("../../gpu/flat/field.wgsl"),
        include_str!("../../gpu/source/read.wgsl"),
        include_str!("../../gpu/interpretation/film_response.wgsl"),
        include_str!("../../gpu/grading/coverage.wgsl"),
        include_str!("../../gpu/grading/colour.wgsl"),
        include_str!("../../gpu/grading/layers.wgsl"),
        include_str!("source_grading.wgsl"),
    ]
    .join("\n")
}
