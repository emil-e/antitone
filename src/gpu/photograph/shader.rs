//! Shared photographic functions and their packed resource bindings.

#[cfg(test)]
mod tests;

pub(crate) fn functions() -> String {
    [
        include_str!("../parameters.wgsl"),
        include_str!("../source/bindings.wgsl"),
        include_str!("../flat/bindings.wgsl"),
        include_str!("../grading/bindings.wgsl"),
        include_str!("pixel.wgsl"),
        include_str!("../working_range.wgsl"),
        include_str!("../interpretation/camera_conversion.wgsl"),
        include_str!("../flat/field.wgsl"),
        include_str!("../source/read.wgsl"),
        include_str!("../interpretation/film_response.wgsl"),
        include_str!("../grading/coverage.wgsl"),
        include_str!("../grading/colour.wgsl"),
        include_str!("../grading/layers.wgsl"),
        include_str!("source_grading.wgsl"),
    ]
    .join("\n")
}
