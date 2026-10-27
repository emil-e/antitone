//! Device-free validation of the composed shaders.
//!
//! Every GPU pipeline is built from several shader files joined into one module, and naga only
//! reports a mismatch between them when a device creates it — in the editor that surfaces as a
//! failed view or thumbnail rather than a failed build. Validating the same text here turns
//! those into ordinary test failures.
#[cfg(feature = "gpu")]
use crate::gpu;

fn validate(name: &str, source: &str, entry_points: &[&str]) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name} does not parse: {error}"));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{name} does not validate: {error}"));
    for entry in entry_points {
        assert!(
            module.entry_points.iter().any(|point| point.name == *entry),
            "{name} is missing the entry point {entry}"
        );
    }
}

#[cfg(feature = "gpu")]
#[test]
fn composed_photographic_kernel_validates() {
    validate(
        "photographic kernel",
        &gpu::ProcessingKernel::module_shader(),
        &["main", "repair_original", "repair_copy", "repair_neutral"],
    );
}

#[cfg(feature = "editor")]
#[test]
fn composed_working_view_validates() {
    validate(
        "working view",
        &gpu::view::view_shader(),
        &["view_main", "inspection_main"],
    );
    validate(
        "image response",
        &gpu::image_mtf::shader(),
        &["horizontal", "vertical"],
    );
    validate("grain", &gpu::grain::shader(), &["field", "apply"]);
}
