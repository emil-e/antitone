use super::super::tests::fixture;
use super::tests::{geometry, run, template};
use super::*;

#[test]
fn retained_crop_is_the_actual_holder_restricted_crop_that_was_validated() {
    let (capture, evidence, mut scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for y in 12..36 {
        for x in 0..12 {
            scene.holder[y * 64 + x] = true;
        }
    }
    let baseline = Areas {
        picture_geometry: Some(geometry(capture.size, [8., 56., 6., 42.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    let roles = scene.roles(&capture);
    let original = auto_crop::propose(capture.area, &baseline).unwrap();
    assert!(nonfilm_fraction(&capture, &roles, original) > 0.02);
    let result = run(
        &capture,
        &evidence,
        &scene,
        &baseline,
        template(capture.size, [8., 56., 6., 42.]),
        4. / 3.,
    );
    let proposal = result.proposal.unwrap();
    assert!(nonfilm_fraction(&capture, &roles, proposal.crop) <= 0.02);
    assert!(
        proposal.crop.center[0] - f64::from(proposal.crop.size[0]) / 2.
            >= f64::from(capture.area.x) + 120.
    );
}

#[test]
fn native_flags_do_not_turn_a_known_holder_limit_into_a_measured_picture_side() {
    let (capture, evidence, mut scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for y in 0..48 {
        for x in 0..8 {
            scene.holder[y * 64 + x] = true;
        }
    }
    let baseline = Areas {
        picture_geometry: Some(geometry(capture.size, [8., 56., 6., 42.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    let result = run(
        &capture,
        &evidence,
        &scene,
        &baseline,
        template(capture.size, [8., 56., 6., 42.]),
        4. / 3.,
    );
    let proposal = result.proposal.unwrap();
    assert!(!proposal.measured[0]);
    assert!(proposal.inferred[0]);
}

#[test]
fn far_median_gap_cannot_hide_near_fragments_that_actually_explain_the_outer_strip() {
    let (mut capture, _, scene) = fixture([72, 72], [20., 52., 20., 52.], None, true);
    for y in 20..52 {
        for x in 0..if y < 28 { 18 } else { 16 } {
            capture.rgb[y * 72 + x] = [8000; 3];
        }
    }
    for x in 20..52 {
        for y in 0..if x < 28 { 18 } else { 16 } {
            capture.rgb[y * 72 + x] = [8000; 3];
        }
    }
    let prior = template(capture.size, [20., 52., 20., 52.]);
    let frame = geometry(capture.size, [20., 52., 20., 52.]);
    let roles = scene.roles(&capture);
    let side = perimeter::measure(&capture, &roles, &frame, 0, prior.exterior[0]);
    assert_eq!(side.neighbor.as_ref().unwrap().gap, 4.);
    assert!(side.neighbor_required);
    let light = light::ExteriorLight::new(&capture, &roles);
    for mode in [Mode::Material, Mode::Joint] {
        assert!(matches!(
            eligible_with(&scene, &capture, &roles, &frame, Some(&prior), &light, mode),
            Err("incompatible_neighbor_axes")
        ));
    }
}

#[test]
fn allowed_rotation_preserves_physical_extents_instead_of_expanding_envelopes() {
    let prior = template([768, 512], [14., 754., 6., 500.]);
    let rotated = prior.frame(0.4);
    assert!(prior.compatible(&rotated));
    assert!((rotated.sides[1].unwrap() - rotated.sides[0].unwrap() - 740.).abs() < 1e-9);
    assert!((rotated.sides[3].unwrap() - rotated.sides[2].unwrap() - 494.).abs() < 1e-9);
    let center = project(
        [
            (rotated.sides[0].unwrap() + rotated.sides[1].unwrap()) / 2.,
            (rotated.sides[2].unwrap() + rotated.sides[3].unwrap()) / 2.,
        ],
        -rotated.angle,
    );
    assert!((center[0] - 384.).abs() < 1e-9);
    assert!((center[1] - 253.).abs() < 1e-9);
}

#[test]
fn physical_extent_mapping_uses_source_units_with_unequal_raster_scales_and_origin() {
    let mut prior = template([768, 512], [14., 754., 6., 500.]);
    prior.scale = [2., 3.];
    prior.width.center *= 2.;
    prior.width.radius *= 2.;
    prior.height.center *= 3.;
    prior.height.radius *= 3.;
    let frame = prior.frame(0.4);
    let source = frame.frame([11, 17, 1536, 1536].into()).unwrap();
    let width = (source[1][0] - source[0][0]).hypot(source[1][1] - source[0][1]);
    let height = (source[3][0] - source[0][0]).hypot(source[3][1] - source[0][1]);
    assert!((width - 1480.).abs() < 1e-8);
    assert!((height - 1482.).abs() < 1e-8);
    assert!(prior.compatible(&frame));
}

#[test]
fn near_duplicate_runner_up_cannot_hide_a_different_near_tied_frame() {
    let a = [Some(10.), Some(750.), Some(10.), Some(500.)];
    let near = [Some(11.), Some(751.), Some(10.), Some(500.)];
    let distinct = [Some(15.), Some(755.), Some(10.), Some(500.)];
    assert!(distinct_competitor(
        4,
        1.,
        a,
        &[(4, 1., a), (4, 0.99, near), (4, 0.95, distinct)]
    ));
    assert!(!distinct_competitor(
        4,
        1.,
        a,
        &[(4, 1., a), (4, 0.99, near), (3, 0.95, distinct)]
    ));
}

#[test]
fn ordered_ratio_below_one_remains_a_valid_detected_physical_shape() {
    let (capture, evidence, scene) = fixture([64, 64], [20., 44., 8., 56.], None, true);
    let baseline = Areas {
        picture_geometry: Some(geometry(capture.size, [20., 44., 8., 56.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    let result = run(
        &capture,
        &evidence,
        &scene,
        &baseline,
        template(capture.size, [20., 44., 8., 56.]),
        0.5,
    );
    let proposal = result.proposal.unwrap();
    assert_eq!(proposal.measured, [true; 4]);
    assert!((physical_ratio(&proposal.physical, capture.area).unwrap() - 0.5).abs() < 1e-9);
}

#[test]
fn a_roll_template_without_observed_picture_transitions_does_not_hallucinate_a_frame() {
    let (mut capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    capture.rgb.fill([20000; 3]);
    let result = run(
        &capture,
        &evidence,
        &scene,
        &Areas::default(),
        template(capture.size, [8., 56., 6., 42.]),
        4. / 3.,
    );
    assert!(result.proposal.is_none());
}

#[test]
fn pose_outlier_does_not_donate_exterior_or_hide_from_rejected_peer_provenance() {
    let mut captures = vec![];
    let mut frames = vec![];
    for id in 0..4 {
        let (capture, _, _) = fixture([64, 48], [8., 56., 6., 42.], None, true);
        let mut frame = geometry(capture.size, [8., 56., 6., 42.]);
        if id == 3 {
            frame.angle = 5.;
        }
        captures.push(capture);
        frames.push((id, frame));
    }
    let (_, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    let roles: Vec<_> = captures
        .iter()
        .map(|capture| scene.roles(capture))
        .collect();
    let learned = Template::fit(&scene, &captures, &roles, &frames).unwrap();
    assert_eq!(learned.donors, vec![0, 1, 2]);
    assert_eq!(learned.rejected, vec![3]);
}

#[test]
fn exterior_reference_stops_at_unknown_material_even_when_base_reappears_farther_out() {
    let mut captures = vec![];
    let mut frames = vec![];
    for id in 0..3 {
        let (mut capture, _, _) = fixture([64, 48], [8., 56., 6., 42.], None, true);
        for y in 0..48 {
            capture.rgb[y * 64 + 6] = [40000, 2000, 20000];
            capture.rgb[y * 64 + 5] = [21400; 3];
        }
        frames.push((id, geometry(capture.size, [8., 56., 6., 42.])));
        captures.push(capture);
    }
    let (_, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    let roles: Vec<_> = captures
        .iter()
        .map(|capture| scene.roles(capture))
        .collect();
    assert_eq!(roles[0][12 * 64 + 6], Role::Unknown);
    assert_eq!(roles[0][12 * 64 + 5], Role::BaseCompatible);
    let learned = Template::fit(&scene, &captures, &roles, &frames).unwrap();
    let exterior = learned.exterior[0].unwrap();
    // Each peer contributes only its first contiguous sample at each declared station.
    assert_eq!(exterior.samples, 3 * 32);
    assert!(
        exterior
            .mean
            .iter()
            .all(|mean| (*mean - 20000f64.log2()).abs() < 1e-12)
    );
}
