use super::super::tests::fixture;
use super::tests::{geometry, run, template};
use super::*;

#[test]
fn observed_rebate_then_known_holder_is_not_continuing_photographic_material() {
    let (mut capture, _, mut scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for x in 0..64 {
        capture.rgb[44 * 64 + x] = [1000; 3];
        for y in 45..48 {
            capture.rgb[y * 64 + x] = [100; 3];
            scene.holder[y * 64 + x] = true;
        }
    }
    let frame = geometry(capture.size, [8., 56., 6., 42.]);
    let prior = template(capture.size, [8., 56., 6., 42.]);
    let evidence = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        3,
        prior.exterior[3],
    );
    assert!(evidence.measured());
    assert_eq!(evidence.base_to_holder, 64);
    assert!(!evidence.contradiction);
    assert!(!evidence.neighbor_required);
}

#[test]
fn photographic_continuation_before_any_base_is_not_excused_by_a_later_holder() {
    let (mut capture, _, mut scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for y in 45..48 {
        for x in 0..64 {
            capture.rgb[y * 64 + x] = [100; 3];
            scene.holder[y * 64 + x] = true;
        }
    }
    let frame = geometry(capture.size, [8., 56., 6., 36.]);
    let prior = template(capture.size, [8., 56., 6., 42.]);
    let evidence = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        3,
        prior.exterior[3],
    );
    assert!(evidence.contradiction);
    assert!(!evidence.measured());
}

#[test]
fn a_holder_explanation_requires_continuity_and_no_rgb_contradiction() {
    let (mut capture, _, mut scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for x in 0..64 {
        capture.rgb[44 * 64 + x] = [1000; 3];
        capture.rgb[45 * 64 + x] = [2000; 3];
        for y in 46..48 {
            capture.rgb[y * 64 + x] = [100; 3];
            scene.holder[y * 64 + x] = true;
        }
    }
    let frame = geometry(capture.size, [8., 56., 6., 42.]);
    let exterior = template(capture.size, [8., 56., 6., 42.]).exterior[3].unwrap();
    let point = perimeter::point(&frame, 3, 0.5);
    let normal = perimeter::normal(frame.angle, 3);
    let mut roles = scene.roles(&capture);
    assert!(!neighbors::holder_after_base(
        &capture, &roles, exterior, point, normal
    ));
    // Removing visible image variation still cannot bridge unavailable material.
    for x in 0..64 {
        capture.rgb[45 * 64 + x] = [500; 3];
        roles[45 * 64 + x] = Role::Unknown;
    }
    assert!(!neighbors::holder_after_base(
        &capture, &roles, exterior, point, normal
    ));
    // A shared holder label cannot override the target's contradictory RGB.
    for x in 0..64 {
        roles[45 * 64 + x] = Role::AttenuatedCompatible;
        capture.rgb[46 * 64 + x] = [20000; 3];
    }
    assert!(!neighbors::holder_after_base(
        &capture, &roles, exterior, point, normal
    ));
}

#[test]
fn separated_photo_without_known_holder_remains_a_neighbor_explanation() {
    let (mut capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for y in 44..48 {
        for x in 0..64 {
            capture.rgb[y * 64 + x] = [1000; 3];
        }
    }
    let frame = geometry(capture.size, [8., 56., 6., 42.]);
    let prior = template(capture.size, [8., 56., 6., 42.]);
    let roles = scene.roles(&capture);
    assert!(!neighbors::holder_after_base(
        &capture,
        &roles,
        prior.exterior[3].unwrap(),
        perimeter::point(&frame, 3, 0.5),
        perimeter::normal(frame.angle, 3)
    ));
    let side = perimeter::measure(&capture, &roles, &frame, 3, prior.exterior[3]);
    assert!(side.neighbor_required);
    assert!(!side.contradiction);
}

#[test]
fn clipped_corners_do_not_disqualify_four_independently_observable_physical_sides() {
    let (mut capture, evidence, scene) = fixture([64, 48], [1., 63., 1., 47.], None, true);
    let mut frame = geometry(capture.size, [1., 63., 1., 47.]);
    frame.angle = 2.;
    for y in 0..48 {
        for x in 0..64 {
            let point = [x as f64 + 0.5, y as f64 + 0.5];
            capture.rgb[y * 64 + x] = if in_hypothesis(point, &frame, None) {
                [8000; 3]
            } else {
                [20000; 3]
            };
        }
    }
    let baseline = Areas {
        picture_geometry: Some(frame.clone()),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    assert!(auto_crop::propose(capture.area, &baseline).is_err());
    let mut prior = template(capture.size, [1., 63., 1., 47.]);
    prior.pose.center = 2.;
    let roles = scene.roles(&capture);
    for side in 0..4 {
        assert!(
            perimeter::measure(&capture, &roles, &frame, side, prior.exterior[side]).measured(),
            "side{side} must be independently visible"
        );
    }
    let report = run(&capture, &evidence, &scene, &baseline, prior, 62. / 46.);
    let proposal = report.proposal.unwrap();
    assert_eq!(proposal.physical.sides, frame.sides);
    assert_eq!(proposal.measured, [true; 4]);
    assert_eq!(proposal.inferred, [false; 4]);
    assert!(proposal.source_clipped);
    assert!(
        proposal
            .crop
            .coverage()
            .unwrap()
            .bounds(capture.area)
            .is_ok()
    );
}
