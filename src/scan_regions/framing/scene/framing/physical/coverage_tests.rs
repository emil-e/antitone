//! Missing observations cannot manufacture positive evidence or supply an independent ratio.
use super::super::tests::{fixture, shape_reference};
use super::tests::{geometry, template};
use super::*;

#[test]
fn a_smooth_ramp_with_heterogeneous_unavailable_controls_is_not_a_boundary() {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 0..96 {
        for x in 0..128 {
            capture.rgb[y * 128 + x] = [(18000.
                * (0.002 * (x as f64 - 112.) + 0.001 * (y as f64 - 48.)).exp2())
            .round() as u16; 3];
        }
    }
    let mut roles = scene.roles(&capture);
    for y in 0..96 {
        if (y / 9) % 4 == 0 {
            roles[y * 128 + 107] = Role::Unknown;
        } else if (y / 9) % 4 == 1 {
            roles[y * 128 + 116] = Role::Unknown;
        }
    }
    let side = Mode::Joint.measure(
        &capture,
        &roles,
        &geometry(capture.size, [16., 112., 12., 84.]),
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.measured());
}

#[test]
fn three_visible_sides_without_an_independent_shape_do_not_complete_the_camera_clipped_side() {
    let bounds = [8., 56., -3., 42.];
    let (capture, evidence, scene) = fixture([64, 48], bounds, None, false);
    let report = evaluate(
        &scene,
        Target {
            capture: &capture,
            evidence: &evidence,
            baseline: &Areas::default(),
            lines: [None, None, None, None],
        },
        &scene.roles(&capture),
        &shape_reference(None),
        Some(template(capture.size, bounds)),
        || Ok(()),
    )
    .unwrap();
    assert!(report.proposal.is_none());
}
