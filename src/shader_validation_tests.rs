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
fn photographic_functions_and_masked_composition_validate() {
    validate(
        "photographic functions",
        &gpu::photograph::shader::functions(),
        &[],
    );
    use gpu::evaluation::*;
    let region = CellRegion::new([0, 0], [4, 4]);
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let base = gpu::repairs::fixtures::image(domain, vec![[1.; 4]; 16]).unwrap();
    let replacement = gpu::repairs::fixtures::image(
        Domain {
            bounds: Some(CellRegion::new([1, 1], [2, 2])),
            ..domain
        },
        vec![[2.; 4]; 4],
    )
    .unwrap();
    let mask =
        crate::dust_repair::SupportMask::new([1, 1], [2, 2], vec![true, false, false, false])
            .unwrap();
    let root = gpu::repairs::definition::compose_images(base, vec![(mask, replacement)]).unwrap();
    let plan = Plan::with_policy(
        root,
        region,
        Budget {
            tile: [4, 4],
            scratch_bytes: 1 << 20,
        },
        Policy::Composed,
    )
    .unwrap();
    let mut computes = 0;
    for work in plan.tiles.iter().flat_map(|tile| &tile.fragment.work) {
        if let Work::Compute { code, entry, .. } = work {
            validate("masked composition", code, &[entry]);
            computes += 1;
        }
    }
    assert!(
        computes > 0,
        "the fixture must validate actual compiler-generated composition code"
    );
}

#[cfg(feature = "editor")]
#[test]
fn composed_working_view_validates() {
    validate(
        "source inspection functions",
        include_str!("gpu/evaluation/inspection.wgsl"),
        &[],
    );
    validate(
        "acquisition copy",
        include_str!("gpu/evaluation/copy_texture.wgsl"),
        &["main"],
    );
    validate(
        "photographic functions",
        &gpu::grading::Grading::shader(),
        &[],
    );
    validate(
        "camera-cell adjustments",
        &format!(
            "{}\n{}",
            gpu::grading::Grading::shader(),
            include_str!("gpu/grading/camera_adjustments.wgsl")
        ),
        &[],
    );
    validate(
        "image response",
        &gpu::image_mtf::shader(),
        &["horizontal", "vertical"],
    );
    validate("grain", &gpu::grain::shader(), &["field", "apply"]);
}
