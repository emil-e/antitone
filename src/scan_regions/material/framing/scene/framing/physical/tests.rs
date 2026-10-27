use super::super::tests::fixture;
use super::*;

pub(super) fn template(size: [u32; 2], bounds: [f64; 4]) -> Template {
    Template {
        size,
        pose: Band {
            center: 0.,
            radius: 0.5,
        },
        sides: bounds.map(|center| Band { center, radius: 3. }),
        centers: [
            Band {
                center: (bounds[0] + bounds[1]) * 0.5,
                radius: 3.,
            },
            Band {
                center: (bounds[2] + bounds[3]) * 0.5,
                radius: 3.,
            },
        ],
        scale: [1.; 2],
        width: Band {
            center: bounds[1] - bounds[0],
            radius: 4.,
        },
        height: Band {
            center: bounds[3] - bounds[2],
            radius: 4.,
        },
        exterior: [Some(Exterior {
            mean: [20000f64.log2(); 3],
            spread: [0.03; 3],
            samples: 100,
        }); 4],
        donors: vec![100, 101, 102],
        rejected: vec![],
    }
}

pub(super) fn geometry(size: [u32; 2], bounds: [f64; 4]) -> PictureGeometry {
    PictureGeometry {
        size,
        angle: 0.,
        sides: bounds.map(Some),
    }
}

pub(super) fn reference(ratio: f64) -> Reference {
    Reference {
        ratio: Some(ratio),
        interval: Some([ratio * 0.998, ratio * 1.002]),
        shape: Some(super::super::super::super::shape::Shape {
            ratio,
            interval: [ratio * 0.998, ratio * 1.002],
            donors: vec![100, 101, 102],
            rejected: vec![],
        }),
        eligible: vec![100, 101, 102],
        donors: vec![100, 101, 102],
        rejected: vec![],
        error: None,
    }
}

pub(super) fn run(
    capture: &Capture,
    evidence: &Evidence,
    scene: &Scene,
    baseline: &Areas,
    prior: Template,
    ratio: f64,
) -> Report {
    evaluate(
        scene,
        Target {
            capture,
            evidence,
            baseline,
            lines: [None, None, None, None],
        },
        &scene.roles(capture),
        &reference(ratio),
        Some(prior),
        || Ok(()),
    )
    .unwrap()
}

#[test]
fn a_short_positive_scene_segment_cannot_outvote_road_continuing_outside_the_rest() {
    let (mut capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    for y in 6..12 {
        for x in 44..56 {
            capture.rgb[y * 64 + x] = [20000; 3];
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [8., 44., 6., 42.]),
        1,
        template(capture.size, [8., 56., 6., 42.]).exterior[1],
    );
    assert!(side.photo > 40);
    assert!(side.contradiction);
    assert!(!side.measured());
    assert!(side.neighbor.is_none());
}

#[test]
fn grossly_wrong_placement_cannot_be_legitimized_by_ratio_or_source_completion() {
    let prior = template([768, 512], [14., 754., 6., 500.]);
    let wrong = geometry([768, 512], [-119., 621., 6., 500.]);
    assert!(!prior.compatible(&wrong));
    let shifted = geometry([768, 512], [15., 755., 7., 501.]);
    assert!(prior.compatible(&shifted));
}

#[test]
fn four_physical_measurements_are_preserved_even_with_a_different_ratio_hint() {
    let (capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
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
        2.7,
    );
    let proposal = result.proposal.unwrap();
    assert_eq!(
        proposal.physical.sides,
        baseline.picture_geometry.unwrap().sides
    );
    assert_eq!(proposal.inferred, [false; 4]);
}

#[test]
fn a_missing_native_location_can_be_measured_near_the_roll_prediction() {
    let (capture, mut evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    evidence.lines.get_mut("right").unwrap().clear();
    let result = run(
        &capture,
        &evidence,
        &scene,
        &Areas::default(),
        template(capture.size, [8., 56., 6., 42.]),
        4. / 3.,
    );
    let proposal = result.proposal.unwrap();
    assert!(proposal.measured.iter().all(|&value| value));
    assert!((proposal.physical.sides[1].unwrap() - 56.).abs() <= 1.);
}

#[test]
fn intentionally_camera_clipped_frame_uses_three_sides_and_independent_shape() {
    let (capture, evidence, scene) = fixture([64, 48], [8., 56., -3., 42.], None, false);
    let mut prior = template(capture.size, [8., 56., 1., 46.]);
    prior.sides[2].radius = 6.;
    prior.sides[3].radius = 6.;
    prior.centers[1].radius = 6.;
    let result = run(
        &capture,
        &evidence,
        &scene,
        &Areas::default(),
        prior,
        48. / 45.,
    );
    let proposal = result.proposal.unwrap();
    assert!(proposal.inferred[2]);
    assert!(!proposal.measured[2]);
    assert!(proposal.physical.sides[2].unwrap() < 0.);
    proposal.crop.region(capture.area).unwrap();
}

#[test]
fn physical_film_cut_is_not_camera_clipping_or_a_picture_anchor() {
    let (capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], Some(50.), false);
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
fn narrow_rebate_before_holder_is_not_skipped_or_counted_as_unavailable_evidence() {
    let (mut capture, _, mut scene) = fixture([64, 48], [12., 60., 6., 42.], None, false);
    for y in 0..48 {
        for x in 0..11 {
            capture.rgb[y * 64 + x] = [100; 3];
            scene.holder[y * 64 + x] = true;
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [12., 60., 6., 42.]),
        0,
        template(capture.size, [12., 60., 6., 42.]).exterior[0],
    );
    assert!(side.measured());
    assert!(!side.contradiction);
}

#[test]
fn faint_or_missing_contrast_on_part_of_a_real_border_does_not_disqualify_it() {
    let (mut capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    for y in 6..26 {
        for x in 50..56 {
            capture.rgb[y * 64 + x] = [20000; 3];
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [8., 56., 6., 42.]),
        1,
        template(capture.size, [8., 56., 6., 42.]).exterior[1],
    );
    assert!(side.measured());
    assert!(side.exterior_supported);
    assert!(!side.contradiction);
}

#[test]
fn image_brighter_on_the_inside_is_a_contradiction_not_a_picture_transition() {
    let (mut capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    for y in 6..42 {
        for x in 8..56 {
            capture.rgb[y * 64 + x] = [40000; 3];
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [8., 56., 6., 42.]),
        0,
        template(capture.size, [8., 56., 6., 42.]).exterior[0],
    );
    assert!(side.contradiction);
    assert!(!side.measured());
}

#[test]
fn adjacent_fragment_is_explained_by_base_gap_and_equal_frame_geometry() {
    let (mut capture, _, scene) = fixture([64, 48], [20., 52., 8., 40.], None, false);
    for y in 8..40 {
        for x in 0..16 {
            capture.rgb[y * 64 + x] = [8000; 3];
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [20., 52., 8., 40.]),
        0,
        template(capture.size, [20., 52., 8., 40.]).exterior[0],
    );
    assert!(side.measured());
    let neighbor = side.neighbor.unwrap();
    assert_eq!(neighbor.axis, 0);
    assert_eq!(neighbor.gap, 4.);
    assert_eq!(
        neighbor.geometry.sides,
        [Some(-16.), Some(16.), Some(8.), Some(40.)]
    );
}

#[test]
fn adjacent_fragment_can_follow_the_other_film_axis_without_orientation_clustering() {
    let (mut capture, _, scene) = fixture([64, 80], [12., 52., 28., 68.], None, false);
    for y in 0..24 {
        for x in 12..52 {
            capture.rgb[y * 64 + x] = [8000; 3];
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [12., 52., 28., 68.]),
        2,
        template(capture.size, [12., 52., 28., 68.]).exterior[2],
    );
    assert!(side.measured());
    let neighbor = side.neighbor.unwrap();
    assert_eq!(neighbor.axis, 1);
    assert_eq!(neighbor.gap, 4.);
}

#[test]
fn no_gap_continuation_cannot_be_relabelled_as_a_neighbor_to_save_a_bad_edge() {
    let (mut capture, _, scene) = fixture([64, 48], [20., 52., 8., 40.], None, false);
    for y in 8..40 {
        for x in 0..20 {
            capture.rgb[y * 64 + x] = [6000; 3];
        }
    }
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [20., 52., 8., 40.]),
        0,
        template(capture.size, [20., 52., 8., 40.]).exterior[0],
    );
    assert!(side.contradiction);
    assert!(side.neighbor.is_none());
}

#[test]
fn a_merged_pair_or_the_previous_frame_is_not_the_roll_conditioned_target() {
    let prior = template([64, 48], [20., 52., 8., 40.]);
    assert!(!prior.compatible(&geometry([64, 48], [-16., 52., 8., 40.])));
    assert!(!prior.compatible(&geometry([64, 48], [-16., 16., 8., 40.])));
}

#[test]
fn template_is_learned_from_complete_peers_with_noise_floor_and_real_scan_variation() {
    let mut captures = vec![];
    let mut frames = vec![];
    for (id, shift) in [0., 1., -1.].into_iter().enumerate() {
        let (capture, _, _) = fixture([64, 48], [8. + shift, 56. + shift, 6., 42.], None, true);
        frames.push((
            id,
            geometry(capture.size, [8. + shift, 56. + shift, 6., 42.]),
        ));
        captures.push(capture);
    }
    let (_, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    let roles: Vec<_> = captures
        .iter()
        .map(|capture| scene.roles(capture))
        .collect();
    let learned = Template::fit(&scene, &captures, &roles, &frames).unwrap();
    assert_eq!(learned.donors, vec![0, 1, 2]);
    assert!(learned.compatible(&geometry([64, 48], [9., 57., 6., 42.])));
    assert!(!learned.compatible(&geometry([64, 48], [25., 73., 6., 42.])));
    assert!(learned.exterior.iter().all(Option::is_some));
}

#[test]
fn off_source_exterior_is_unknown_not_contradictory_or_a_measured_camera_side() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., -3., 42.], None, false);
    let side = perimeter::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [8., 56., -3., 42.]),
        2,
        template(capture.size, [8., 56., -3., 42.]).exterior[2],
    );
    assert_eq!(side.available, 0);
    assert!(!side.contradiction);
    assert!(!side.measured());
}

#[test]
fn a_target_or_completed_frame_cannot_silently_donate_its_own_geometry() {
    let mut captures = vec![];
    for _ in 0..4 {
        captures.push(fixture([64, 48], [8., 56., 6., 42.], None, true).0);
    }
    let model = RollModel::fit(&captures).unwrap();
    let (_, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    let prepared = Prepared::new(&scene, &captures);
    let mut reference = reference(4. / 3.);
    reference.donors = vec![0];
    let result = super::run(
        Context {
            scene: &scene,
            captures: &captures,
            model: &model,
            prepared: &prepared,
            reference: &reference,
            index: 0,
        },
        Target {
            capture: &captures[0],
            evidence: &evidence,
            baseline: &Areas::default(),
            lines: [None, None, None, None],
        },
        || Ok(()),
    );
    assert!(result.is_err());
}

#[test]
fn distant_dark_exterior_is_not_forced_into_simultaneous_neighbor_axes() {
    let (mut capture, evidence, scene) = fixture([72, 72], [20., 52., 20., 52.], None, true);
    for y in 20..52 {
        for x in 0..16 {
            capture.rgb[y * 72 + x] = [8000; 3];
        }
    }
    for y in 0..16 {
        for x in 20..52 {
            capture.rgb[y * 72 + x] = [8000; 3];
        }
    }
    let baseline = Areas {
        picture_geometry: Some(geometry(capture.size, [20., 52., 20., 52.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    let result = run(
        &capture,
        &evidence,
        &scene,
        &baseline,
        template(capture.size, [20., 52., 20., 52.]),
        1.,
    );
    assert!(result.proposal.is_some());
    assert!(result.neighbors.is_empty());
    assert!(result.neighbor_candidates.len() >= 2);
}

#[test]
fn a_narrow_visible_base_gap_can_explain_a_real_neighbor_inside_the_outer_strip() {
    let (mut capture, evidence, scene) = fixture([64, 48], [20., 52., 8., 40.], None, true);
    for y in 8..40 {
        for x in 0..18 {
            capture.rgb[y * 64 + x] = [8000; 3];
        }
    }
    let baseline = Areas {
        picture_geometry: Some(geometry(capture.size, [20., 52., 8., 40.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    let result = run(
        &capture,
        &evidence,
        &scene,
        &baseline,
        template(capture.size, [20., 52., 8., 40.]),
        1.,
    );
    assert!(result.proposal.is_some());
    assert_eq!(result.neighbors.len(), 1);
    assert_eq!(result.neighbors[0].axis, 0);
    assert_eq!(result.neighbors[0].gap, 2.);
}
