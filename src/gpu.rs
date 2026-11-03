//! GPU device setup and the modules that own calculations, resources and presentation.

#[cfg(any(feature = "render", feature = "project"))]
pub(crate) mod aces;
#[cfg(feature = "editor")]
pub(crate) mod analysis;
pub mod evaluation;
#[cfg(all(feature = "render", any(feature = "raw", feature = "ui")))]
pub(crate) mod execution;
pub mod flat;
pub mod grading;
pub mod interpretation;
pub mod lut3d;
pub mod photograph;
#[cfg(all(
    feature = "raw",
    feature = "render",
    feature = "project",
    any(feature = "editor", feature = "export")
))]
pub(crate) mod repair_source;
#[cfg(all(feature = "raw", feature = "render"))]
pub(crate) mod rgb16;
#[cfg(all(test, feature = "render"))]
pub(crate) mod test_support;

pub(crate) mod grain;
pub(crate) mod image_mtf;
#[cfg(any(
    feature = "editor",
    all(feature = "ui", feature = "render"),
    all(feature = "raw", feature = "render")
))]
pub(crate) mod pixel_pipeline;
pub(crate) mod raster;

pub mod input;
pub mod parameters;
pub(crate) mod repairs;
pub mod source;
pub mod submission;
#[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
pub(crate) mod timestamp_ledger;
#[cfg(feature = "editor")]
pub(crate) mod view;

/// The graphics API this platform presents and computes with, from the one `sys` selection.
pub fn backends() -> wgpu::Backends {
    crate::sys::backends()
}

/// An instance with this platform's graphics API and shader-compiler policy.
pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::sys::backends(),
        backend_options: crate::sys::backend_options(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

#[cfg(test)]
mod flat_tests;
#[cfg(test)]
mod input_grading_tests;
#[cfg(test)]
mod tests;
