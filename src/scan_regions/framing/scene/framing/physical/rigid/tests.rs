use super::super::super::tests::{fixture, line};
use super::super::tests::{geometry as rectangle, template};
use super::*;

pub(super) fn calibration_for(capture: &Capture, bounds: [f64; 4]) -> Calibration {
    let mut prior = template(capture.size, bounds);
    prior.scale = [
        f64::from(capture.area.width) / f64::from(capture.size[0]),
        f64::from(capture.area.height) / f64::from(capture.size[1]),
    ];
    prior.width.center *= prior.scale[0];
    prior.width.radius *= prior.scale[0];
    prior.height.center *= prior.scale[1];
    prior.height.radius *= prior.scale[1];
    Calibration {
        source_size: [prior.width.center, prior.height.center],
        template: Some(prior),
        donors: vec![100, 101, 102],
        excluded_targets: vec![0],
    }
}

fn evaluate(
    capture: &Capture,
    evidence: &Evidence,
    scene: &Scene,
    calibration: &Calibration,
) -> Report {
    search(
        scene,
        Target {
            capture,
            evidence,
            baseline: &Areas::default(),
            lines: [const { None }; 4],
        },
        &scene.roles(capture),
        calibration,
        || Ok(()),
    )
    .unwrap()
}

pub(super) fn content_is_preserved(capture: &Capture, choice: &Choice, truth: &PictureGeometry) {
    let [l, r, t, b] = truth.sides.map(Option::unwrap);
    // These declared high-density interior points are beyond the authored2px taper.
    // Do not demand one precise fitted line inside that taper.
    for p in [
        [l + 2., t + 2.],
        [r - 2., t + 2.],
        [r - 2., b - 2.],
        [l + 2., b - 2.],
    ] {
        assert!(
            in_hypothesis(project(p, -truth.angle), &choice.physical, None),
            "wanted interior {p:?} lost"
        );
    }
    let wanted = [
        [l + 2., t + 2.],
        [r - 2., t + 2.],
        [r - 2., b - 2.],
        [l + 2., b - 2.],
    ]
    .map(|p| {
        let p = project(p, -truth.angle);
        [
            f64::from(capture.area.x)
                + p[0] * f64::from(capture.area.width) / f64::from(capture.size[0]),
            f64::from(capture.area.y)
                + p[1] * f64::from(capture.area.height) / f64::from(capture.size[1]),
        ]
    });
    let score = auto_crop::diagnostics::score(choice.crop, &[wanted], true).unwrap();
    let error = serde_json::to_value(score).unwrap()["error_source_pixel_area"]
        .as_f64()
        .unwrap();
    assert!(
        error < 1e-6,
        "actual visible crop loses authored wanted area: {error}"
    );
    for p in [
        [l - 4., (t + b) * 0.5],
        [r + 4., (t + b) * 0.5],
        [(l + r) * 0.5, t - 4.],
        [(l + r) * 0.5, b + 4.],
    ] {
        assert!(
            !in_hypothesis(project(p, -truth.angle), &choice.physical, None),
            "far exterior {p:?} admitted"
        );
    }
}

#[test]
fn adjacent_pairs_place_one_fixed_source_size_at_rotated_nonzero_origin() {
    let (capture, _, _) = fixture([128, 96], [16., 112., 12., 84.], None, true);
    for pose in [-0.25_f64, 0., 0.25] {
        let (s, c) = pose.to_radians().sin_cos();
        for vertical in 0..2 {
            for horizontal in 2..4 {
                let bounds = [16., 112., 12., 84.];
                let mut v = line((bounds[vertical] - s * 48.) / c, true);
                v.angle_degrees = -pose;
                let mut h = line((bounds[horizontal] + s * 64.) / c, true);
                h.angle_degrees = pose;
                let frame = frame_from_pair(
                    &capture,
                    [960., 720.],
                    vertical,
                    horizontal,
                    [
                        native_bound(&v, vertical, capture.size, pose).unwrap(),
                        native_bound(&h, horizontal, capture.size, pose).unwrap(),
                    ],
                    pose,
                )
                .unwrap();
                let source = frame.frame(capture.area).unwrap();
                let q0 = [c * 16. - s * 12., s * 16. + c * 12.];
                assert!((source[0][0] - (11. + 10. * q0[0])).abs() < 1e-8);
                assert!((source[0][1] - (17. + 10. * q0[1])).abs() < 1e-8);
                assert!(
                    ((source[1][0] - source[0][0]).hypot(source[1][1] - source[0][1]) - 960.).abs()
                        < 1e-8
                );
                assert!(
                    ((source[3][0] - source[0][0]).hypot(source[3][1] - source[0][1]) - 720.).abs()
                        < 1e-8
                );
            }
        }
    }
    assert!(frame_from_pair(&capture, [960., 720.], 0, 1, [16., 112.], 0.).is_none());
    assert!(frame_from_pair(&capture, [960., 720.], 2, 3, [12., 84.], 0.).is_none());
    let mut unobserved = line(16., true);
    unobserved.span_intervals = Some(vec![]);
    assert!(native_bound(&unobserved, 0, capture.size, 0.).is_none());
}

#[test]
fn physical_dimensions_use_target_source_scale_not_the_donors_raster_scale() {
    let (mut capture, _, _) = fixture([128, 96], [16., 112., 12., 84.], None, true);
    capture.area = [13, 29, 960, 1920].into();
    let frame = frame_from_pair(&capture, [900., 700.], 1, 3, [107., 89.], 0.37).unwrap();
    let q = frame.frame(capture.area).unwrap();
    assert!(((q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]) - 900.).abs() < 1e-8);
    assert!(((q[3][0] - q[0][0]).hypot(q[3][1] - q[0][1]) - 700.).abs() < 1e-8);
}

fn candidate(frame: PictureGeometry, score: f64, gradient: f64) -> Candidate {
    let (capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, true);
    let calibration = calibration_for(&capture, [16., 112., 12., 84.]);
    let mut candidate = assess(
        &scene,
        &capture,
        &scene.roles(&capture),
        &calibration,
        rectangle(capture.size, [16., 112., 12., 84.]),
        [true, false, true, false],
    )
    .unwrap();
    // Selection tests start with qualified alternatives; physical qualification is
    // exercised separately by full search and completed-companion controls.
    candidate.choice.crop = visible_crop(capture.area, &frame).unwrap();
    candidate.choice.physical = frame;
    candidate.anchor_score = score;
    candidate.all_side_gradients[1].mean = gradient;
    candidate
}

#[test]
fn independent_normal_motion_is_not_combined_diagonal_corner_error() {
    let a = rectangle([128, 96], [16., 112., 12., 84.]);
    let b = rectangle([128, 96], [18.5, 114.5, 14.5, 86.5]);
    assert!((boundary_distance(&a, &b) - 2.5).abs() < 1e-12);
    assert!((boundary_distance(&b, &a) - 2.5).abs() < 1e-12);
    assert!(2.5f64.hypot(2.5) > NEAR_BOUNDARY);
    let mut report = Report::default();
    select(
        &mut report,
        vec![candidate(a, 1., 0.), candidate(b, 0.99, 0.)],
    );
    assert!(report.proposal.is_some());
    assert!(!report.ambiguity);
}

#[test]
fn rotated_equivalence_checks_endpoint_normal_motion_not_stored_bounds_or_midpoints() {
    let a = rectangle([128, 96], [16., 112., 12., 84.]);
    for (angle, equivalent) in [(1_f64, true), (2_f64, false)] {
        let mut b = a.clone();
        b.angle = angle;
        let (s, c) = angle.to_radians().sin_cos();
        // For these positive coordinates/angles, the largest symmetric residual is
        // the bottom line at x112:112 sin(angle)+84(1-cos(angle)). Stored UV bounds
        // are identical, and checking only the line midpoint misses the2deg case.
        let expected = 112. * s + 84. * (1. - c);
        assert!((boundary_distance(&a, &b) - expected).abs() < 1e-10);
        assert!((boundary_distance(&b, &a) - expected).abs() < 1e-10);
        assert!(64. * s + 84. * (1. - c) < NEAR_BOUNDARY);
        let mut report = Report::default();
        select(
            &mut report,
            vec![candidate(a.clone(), 1., 0.), candidate(b, 0.99, 0.)],
        );
        assert_eq!(report.proposal.is_some(), equivalent);
        assert_eq!(report.ambiguity, !equivalent);
    }
}

#[test]
fn nearby_runner_up_and_nontransitive_chains_cannot_hide_a_distinct_frame() {
    let make = |dx| rectangle([128, 96], [16. + dx, 112. + dx, 12., 84.]);
    let mut report = Report::default();
    select(
        &mut report,
        vec![
            candidate(make(0.), 1., 0.),
            candidate(make(2.), 0.99, 0.),
            candidate(make(4.), 0.95, 0.),
        ],
    );
    assert!(report.proposal.is_none());
    assert!(report.ambiguity);
    assert!(
        report
            .distinct_near_ties
            .iter()
            .any(|v| (v.boundary_distance - 4.).abs() < 1e-12)
    );
}

#[test]
fn selection_is_order_independent_and_inferred_side_evidence_only_breaks_exact_primary_ties() {
    let a = candidate(rectangle([128, 96], [16., 112., 12., 84.]), 1., 0.);
    let b = candidate(rectangle([128, 96], [16.5, 112.5, 12., 84.]), 1., 1.);
    let lower = candidate(rectangle([128, 96], [17., 113., 12., 84.]), 0.999, 100.);
    for values in [
        vec![a.clone(), b.clone(), lower.clone()],
        vec![lower.clone(), b.clone(), a.clone()],
        vec![b.clone(), a.clone(), lower.clone()],
    ] {
        let mut report = Report::default();
        select(&mut report, values);
        let choice = report.proposal.unwrap();
        assert_eq!(choice.physical.sides, b.choice.physical.sides);
        assert_eq!(choice.measured, [true, false, true, false]);
        assert_eq!(choice.inferred, [false, true, false, true]);
    }
}

#[test]
fn one_batch_calibration_excludes_all_targets_and_is_identical_for_each_target() {
    let (mut captures, evidence): (Vec<_>, Vec<_>) = (0..11)
        .map(|id| {
            let bounds = if id == 9 {
                [10., 54., 6., 42.]
            } else {
                [8., 56., 6., 42.]
            };
            let (capture, evidence, _) = fixture([64, 48], bounds, None, true);
            (capture, evidence)
        })
        .unzip();
    let excluded = [0, 2, 4, 6, 7, 8, 10];
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    let prepared = Prepared::new(&scene, &captures);
    // Targets really are complete eligible donors before exclusion; peer9 is a
    // complete ratio outlier, so rejected as well as accepted IDs need remapping.
    let reference = reference(&scene, &captures, &model, &prepared, 0, || Ok(())).unwrap();
    assert!(reference.eligible.contains(&2));
    let check_peers = [1, 3, 5, 9].map(|id| &captures[id]);
    let check_model = RollModel::fit_refs(&check_peers).unwrap();
    let outlier = check_model
        .areas(3, check_peers[3], check_peers[3].size, &check_peers[3].rgb)
        .unwrap();
    assert!(
        auto_crop::propose(check_peers[3].area, &outlier).is_ok(),
        "ratio-outlier control must reach donor admission: {}",
        serde_json::to_value(&outlier).unwrap()
    );
    let report = calibrate(&captures, &excluded, || Ok(())).unwrap();
    assert_eq!(report.reference.eligible, vec![1, 3, 5, 9]);
    assert_eq!(report.reference.donors, vec![1, 3, 5]);
    assert_eq!(report.reference.rejected, vec![9]);
    let calibrated = report.model.unwrap();
    assert_eq!(calibrated.source_size, [480., 360.]);
    assert_eq!(calibrated.donors, vec![1, 3, 5]);
    assert_eq!(calibrated.template.as_ref().unwrap().donors, vec![1, 3, 5]);
    let frozen = serde_json::to_value(&calibrated).unwrap();
    for index in [0, 2] {
        let context = Context {
            scene: &scene,
            captures: &captures,
            model: &model,
            prepared: &prepared,
            reference: &reference,
            index,
        };
        let baseline = model
            .areas(
                index,
                &captures[index],
                captures[index].size,
                &captures[index].rgb,
            )
            .unwrap();
        let target = Target {
            capture: &captures[index],
            evidence: &evidence[index],
            baseline: &baseline,
            lines: selected_lines(&model, index, &captures[index]),
        };
        assert!(
            run(&context, target, &calibrated, || Ok(()))
                .unwrap()
                .proposal
                .is_some()
        );
        let context = Context {
            index: 1,
            ..context
        };
        let bad = Target {
            capture: &captures[1],
            evidence: &evidence[1],
            baseline: &baseline,
            lines: [const { None }; 4],
        };
        assert!(run(&context, bad, &calibrated, || Ok(())).is_err());
    }
    let unchanged = |captures: &[Capture]| {
        let changed = calibrate(captures, &excluded, || Ok(()))
            .unwrap()
            .model
            .unwrap();
        assert_eq!(serde_json::to_value(changed).unwrap(), frozen);
    };
    // Pixel-only corruption retains valid matching source geometry and cue groups.
    // A full-roll native model demonstrably loses the clear peer measurement.
    for &id in &excluded {
        captures[id].rgb.fill([100; 3]);
    }
    unchanged(&captures);
    let full = RollModel::fit(&captures).unwrap();
    let contaminated = full
        .areas(1, &captures[1], captures[1].size, &captures[1].rgb)
        .unwrap();
    assert!(
        auto_crop::propose(captures[1].area, &contaminated).is_err(),
        "valid pixel-only control must distinguish full-roll from peer-only fitting"
    );
    for &id in &excluded {
        captures[id] = fixture([64, 48], [8., 56., 6., 42.], None, true).0;
    }
    // Cue-only changes cannot select a different donor family through pooled layout.
    for &id in &excluded {
        captures[id].group = fixture([64, 48], [2., 60., 1., 47.], None, true).0.group;
    }
    unchanged(&captures);
    for &id in &excluded {
        captures[id] = fixture([64, 48], [8., 56., 6., 42.], None, true).0;
    }
    captures[0].area = [17, 29, 999, 777].into();
    unchanged(&captures);
    let mut too_few = excluded.to_vec();
    too_few.extend([5, 9]);
    assert!(
        calibrate(&captures, &too_few, || Ok(()))
            .unwrap()
            .model
            .is_none()
    );
}

#[test]
fn inferred_weak_clipped_or_unqualified_geometry_cannot_donate_size() {
    let (mut capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    let prior = calibration_for(&capture, [8., 56., 6., 42.]);
    let complete = Areas {
        picture_geometry: Some(rectangle(capture.size, [8., 56., 6., 42.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    assert_eq!(
        calibration::check_donor(
            &scene,
            &capture,
            &scene.roles(&capture),
            &complete,
            prior.template.as_ref().unwrap()
        )
        .unwrap(),
        [480., 360.]
    );
    let result = evaluate(&capture, &evidence, &scene, &prior);
    let choice = result.proposal.unwrap();
    assert_eq!(choice.measured.iter().filter(|&&v| v).count(), 2);
    let inferred = Areas {
        picture_geometry: Some(choice.physical),
        picture_observed: choice.measured,
        ..Areas::default()
    };
    assert!(
        calibration::check_donor(
            &scene,
            &capture,
            &scene.roles(&capture),
            &inferred,
            prior.template.as_ref().unwrap()
        )
        .is_err()
    );
    let mut weak = complete.clone();
    weak.picture_weak[0] = true;
    assert!(
        calibration::check_donor(
            &scene,
            &capture,
            &scene.roles(&capture),
            &weak,
            prior.template.as_ref().unwrap()
        )
        .is_err()
    );
    let mut clipped = complete.clone();
    clipped.picture_geometry.as_mut().unwrap().sides[2] = Some(-1.);
    assert_eq!(
        calibration::check_donor(
            &scene,
            &capture,
            &scene.roles(&capture),
            &clipped,
            prior.template.as_ref().unwrap()
        )
        .unwrap_err(),
        "camera_clipped_donor"
    );
    capture.rgb.fill([20000; 3]);
    assert!(
        calibration::check_donor(
            &scene,
            &capture,
            &scene.roles(&capture),
            &complete,
            prior.template.as_ref().unwrap()
        )
        .is_err()
    );
}

#[test]
fn completed_companions_requalify_both_rigid_anchors() {
    let bounds = [8., 44., 6., 42.];
    let (mut capture, _, scene) = fixture([96, 64], bounds, None, false);
    capture.rgb.fill([20000; 3]);
    for y in 12..36 {
        for x in 8..12 {
            capture.rgb[y * 96 + x] = [6000; 3];
        }
    }
    for y in 6..42 {
        for x in 48..96 {
            capture.rgb[y * 96 + x] = [6000; 3];
        }
    }
    let mut calibrated = calibration_for(&capture, bounds);
    calibrated.template = calibration_for(&capture, [8., 88., 6., 42.]).template;
    let roles = scene.roles(&capture);
    let provisional = calibrated.template.as_ref().unwrap().frame(0.);
    assert!(
        Mode::Joint
            .measure(
                &capture,
                &roles,
                &provisional,
                2,
                calibrated.template.as_ref().unwrap().exterior[2]
            )
            .measured()
    );
    let frame = frame_from_pair(&capture, calibrated.source_size, 0, 2, [8., 6.], 0.).unwrap();
    let validation = validation_template(&capture, &calibrated, &frame);
    let (_, sides) = eligible_with(
        &scene,
        &capture,
        &roles,
        &frame,
        validation.as_ref(),
        &light::ExteriorLight::new(&capture, &roles),
        Mode::Joint,
    )
    .unwrap();
    assert!(sides[0].measured());
    assert!(!sides[2].measured());
    assert!(
        assess(
            &scene,
            &capture,
            &roles,
            &calibrated,
            frame,
            [true, false, true, false],
        )
        .is_err()
    );
}

#[test]
fn adaptive_weak_partial_and_missing_sides_keep_content_without_four_measurement_requirement() {
    for seed in [15001, 15002, 15003, 15004, 71801, 71802, 71803, 71804] {
        for partial in [false, true] {
            let (capture, evidence, scene, truth) = if partial {
                weak_fixtures::patch_surface(seed, 0.04, true, 3)
            } else {
                weak_fixtures::surface(seed, 0.04, true, false)
            };
            let calibrated = calibration_for(&capture, [16., 112., 12., 84.]);
            let report = evaluate(&capture, &evidence, &scene, &calibrated);
            let choice = report
                .proposal
                .unwrap_or_else(|| panic!("weak/partial seed{seed}, partial{partial} abstained"));
            content_is_preserved(&capture, &choice, &truth);
            assert_eq!(choice.measured.iter().filter(|&&v| v).count(), 2);
        }
        let (capture, evidence, scene, truth) = weak_fixtures::surface(seed, 0., true, false);
        let report = evaluate(
            &capture,
            &evidence,
            &scene,
            &calibration_for(&capture, [16., 112., 12., 84.]),
        );
        if let Some(choice) = report.proposal {
            content_is_preserved(&capture, &choice, &truth);
        }
        let (capture, evidence, scene, _) = weak_fixtures::surface(seed, 0., true, true);
        assert!(
            evaluate(
                &capture,
                &evidence,
                &scene,
                &calibration_for(&capture, [16., 112., 12., 84.])
            )
            .proposal
            .is_none()
        );
    }
}

#[test]
fn a_stronger_internal_line_cannot_move_the_fixed_frame_inward_and_lose_authored_content() {
    for seed in [15001, 15002, 15003, 15004, 71801, 71802, 71803, 71804] {
        let (capture, evidence, scene, truth) = known_limitations::interior_line(seed);
        let report = evaluate(
            &capture,
            &evidence,
            &scene,
            &calibration_for(&capture, [16., 112., 12., 84.]),
        );
        assert!(
            report.qualified > 0,
            "must exercise selection, not absent discovery"
        );
        if let Some(choice) = report.proposal {
            content_is_preserved(&capture, &choice, &truth);
        }
    }
}

#[test]
fn camera_and_film_availability_do_not_replace_nominal_edges_but_unanchored_frames_abstain() {
    let (capture, evidence, scene) = fixture([128, 96], [16., 112., -8., 64.], None, true);
    let report = evaluate(
        &capture,
        &evidence,
        &scene,
        &calibration_for(&capture, [16., 112., 12., 84.]),
    );
    let choice = report.proposal.unwrap();
    assert!(choice.source_clipped);
    assert!(choice.inferred[2] && !choice.measured[2]);
    assert_eq!(choice.physical.sides[2], Some(-8.));
    choice.crop.region(capture.area).unwrap();
    let (capture, evidence, scene) = fixture([128, 96], [16., 112., 12., 84.], Some(100.), true);
    let cut = evaluate(
        &capture,
        &evidence,
        &scene,
        &calibration_for(&capture, [16., 112., 12., 84.]),
    )
    .proposal
    .expect("a film end does not move the inferred original frame");
    assert_eq!(cut.physical.sides[1], Some(112.));
    assert!(!cut.measured[1] && cut.inferred[1]);
    assert_eq!(
        cut.crop.region(capture.area).unwrap(),
        SourceRegion::from([171, 137, 960, 720])
    );
    for right in [false, true] {
        let (mut capture, mut evidence, scene) =
            fixture([128, 96], [16., 112., 12., 84.], None, true);
        for y in 0..96 {
            for x in 0..128 {
                capture.rgb[y * 128 + x] = if x >= 16 && (!right || x < 112) {
                    [6000; 3]
                } else {
                    [20000; 3]
                };
            }
        }
        evidence.lines.get_mut("top").unwrap().clear();
        evidence.lines.get_mut("bottom").unwrap().clear();
        assert!(
            evaluate(
                &capture,
                &evidence,
                &scene,
                &calibration_for(&capture, [16., 112., 12., 84.])
            )
            .proposal
            .is_none()
        );
    }
}

#[test]
fn unavailable_material_cannot_hide_picture_continuation_on_the_surviving_inferred_edge() {
    let bounds = [16., 112., 12., 84.];
    let (mut capture, _, scene) = fixture([128, 96], bounds, Some(100.), true);
    let calibration = calibration_for(&capture, bounds);
    let frame = rectangle(capture.size, bounds);
    let roles = scene.roles(&capture);
    assert!(
        assess(
            &scene,
            &capture,
            &roles,
            &calibration,
            frame.clone(),
            [true, false, true, false]
        )
        .is_ok(),
        "pure missing film does not veto the nominal frame"
    );
    // Three quarters of the inferred right edge are unavailable. Along the surviving
    // quarter the picture actually continues past that edge; both facts must survive.
    for y in 12..30 {
        for x in 100..128 {
            capture.rgb[y * 128 + x] = [6000; 3];
        }
    }
    let roles = scene.roles(&capture);
    let exterior = calibration.template.as_ref().unwrap().exterior;
    assert!(
        Mode::Joint
            .measure(&capture, &roles, &frame, 0, exterior[0])
            .measured()
    );
    assert!(
        Mode::Joint
            .measure(&capture, &roles, &frame, 2, exterior[2])
            .measured()
    );
    let side = Mode::Joint.measure(&capture, &roles, &frame, 1, exterior[1]);
    assert!(
        side.support.contradicted(),
        "fixture must exercise unavailable material as well as picture continuation"
    );
    assert!(side.directional.as_ref().unwrap().contradictory_stations >= 12);
    assert!(
        matches!(
            assess(
                &scene,
                &capture,
                &roles,
                &calibration,
                frame,
                [true, false, true, false]
            ),
            Err("perimeter_contradiction")
        ),
        "a fit supported by two real anchors still cannot contradict surviving picture evidence"
    );
}

#[test]
fn candidate_search_propagates_cancellation_instead_of_publishing_partial_candidates() {
    let (capture, evidence, scene) = fixture([128, 96], [16., 112., 12., 84.], None, true);
    let calibration = calibration_for(&capture, [16., 112., 12., 84.]);
    let calls = std::cell::Cell::new(0);
    let target = Target {
        capture: &capture,
        evidence: &evidence,
        baseline: &Areas::default(),
        lines: [const { None }; 4],
    };
    let result = search(&scene, target, &scene.roles(&capture), &calibration, || {
        calls.set(calls.get() + 1);
        if calls.get() >= 3 {
            Err("cancelled".into())
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(reason) if reason == "cancelled"));
}

#[test]
fn direct_gradient_sign_constant_and_unavailable_pixels_have_independent_references() {
    let (mut capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    for y in 0..48 {
        for x in 0..64 {
            capture.rgb[y * 64 + x] =
                [(12000. * (0.01 * x as f64 - 0.02 * y as f64).exp2()).round() as u16; 3];
        }
    }
    let gradient = projection::gradient(&capture, [30.5, 20.5]).unwrap();
    assert!((gradient[0] - 0.01).abs() < 0.0002);
    assert!((gradient[1] + 0.02).abs() < 0.0002);
    for side in 0..4 {
        let outward = perimeter::normal(0., side).map(|v| -v);
        let expected = [-0.01, 0.01, 0.02, -0.02][side];
        assert!((gradient[0] * outward[0] + gradient[1] * outward[1] - expected).abs() < 0.0002);
    }
    let frame = rectangle(capture.size, [8., 56., 6., 42.]);
    let evidence = Mode::Joint.measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [8., 56., 6., 42.]).exterior[1],
    );
    let projected = projection::measure(&capture, &frame, 1, &evidence);
    let stations = &evidence.directional.as_ref().unwrap().stations;
    assert!(
        projected.available >= 12,
        "nonzero scorer integration must be exercised"
    );
    assert_eq!(projected.available, stations.len());
    let expected = stations
        .iter()
        .map(|station| (0.01 / station.noise.hypot(station.tangent)).clamp(-4., 4.))
        .sum::<f64>()
        / stations.len() as f64;
    let rounding_bound = stations
        .iter()
        .map(|station| 0.0002 / station.noise.hypot(station.tangent))
        .sum::<f64>()
        / stations.len() as f64;
    assert!(projected.mean > 0.);
    assert!((projected.mean - expected).abs() <= rounding_bound);
    capture.rgb.fill([20000; 3]);
    assert_eq!(projection::gradient(&capture, [30.5, 20.5]), Some([0., 0.]));
    assert!(projection::gradient(&capture, [0.5, 20.5]).is_none());
    capture.rgb[20 * 64 + 31] = [65535; 3];
    assert!(projection::gradient(&capture, [30.5, 20.5]).is_none());
    let frame = rectangle(capture.size, [8., 56., -8., 42.]);
    let side = Mode::Joint.measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        2,
        template(capture.size, [8., 56., 6., 42.]).exterior[2],
    );
    let projection = projection::measure(&capture, &frame, 2, &side);
    assert_eq!(projection.available, 0);
    assert_eq!(projection.mean, 0.);
}
