//! Shared photographic functions and their packed resource bindings.

pub(crate) fn functions() -> String {
    [
        include_str!("../parameters.wgsl"),
        include_str!("grading/bindings.wgsl"),
        include_str!("pixel.wgsl"),
        include_str!("working_range.wgsl"),
        include_str!("interpretation/camera_conversion.wgsl"),
        include_str!("interpretation/film_response.wgsl"),
        include_str!("grading/coverage.wgsl"),
        include_str!("grading/colour.wgsl"),
        include_str!("grading/layers.wgsl"),
    ]
    .join("\n")
}
