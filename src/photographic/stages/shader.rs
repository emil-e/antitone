//! Shared photographic functions and their packed resource bindings.

pub(crate) fn functions() -> String {
    [
        include_str!("../../gpu/parameters.wgsl"),
        include_str!("../../gpu/source/bindings.wgsl"),
        include_str!("../../gpu/flat/bindings.wgsl"),
        include_str!("grading/bindings.wgsl"),
        include_str!("pixel.wgsl"),
        include_str!("../../gpu/working_range.wgsl"),
        include_str!("interpretation/camera_conversion.wgsl"),
        include_str!("../../gpu/flat/field.wgsl"),
        include_str!("../../gpu/source/read.wgsl"),
        include_str!("interpretation/film_response.wgsl"),
        include_str!("grading/coverage.wgsl"),
        include_str!("grading/colour.wgsl"),
        include_str!("grading/layers.wgsl"),
        include_str!("grading/source_grading.wgsl"),
    ]
    .join("\n")
}
