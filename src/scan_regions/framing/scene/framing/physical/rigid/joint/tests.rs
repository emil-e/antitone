use super::super::super::super::tests::{fixture, line};
use super::super::super::tests::geometry as rectangle;
use super::super::tests::{calibration_for, content_is_preserved};
use super::*;

fn baseline(capture: &Capture, bounds: [f64; 4], observed: [bool; 4]) -> Areas {
    Areas {
        picture_geometry: Some(rectangle(capture.size, bounds)),
        picture_observed: observed,
        ..Areas::default()
    }
}

fn target<'a>(capture: &'a Capture, evidence: &'a Evidence, areas: &'a Areas) -> Target<'a> {
    Target {
        capture,
        evidence,
        baseline: areas,
        lines: std::array::from_fn(|side| {
            evidence.lines[crate::scan_regions::lines::SIDES[side]]
                .first()
                .cloned()
        }),
    }
}

#[test]
fn opposite_observation_disagreement_is_balanced_without_changing_size() {
    let (capture, evidence, _) = fixture([128, 96], [16., 110., 12., 84.], None, true);
    let areas = baseline(&capture, [16., 110., 12., 84.], [true; 4]);
    let data = observations(&target(&capture, &evidence, &areas)).unwrap();
    let fit = solve(&capture, [960., 720.], data);
    assert!((fit.geometry.sides[0].unwrap() - 15.).abs() < 1e-9);
    assert!((fit.geometry.sides[1].unwrap() - 111.).abs() < 1e-9);
    assert!((fit.rms - 0.5_f64.sqrt()).abs() < 1e-9);
    for missing in 0..4 {
        let mut partial = areas.clone();
        partial.picture_observed[missing] = false;
        let fit = solve(
            &capture,
            [960., 720.],
            observations(&target(&capture, &evidence, &partial)).unwrap(),
        );
        assert!(!fit.measured[missing]);
        let q = fit.geometry.frame(capture.area).unwrap();
        assert!(((q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]) - 960.).abs() < 1e-8);
        assert!(((q[3][0] - q[0][0]).hypot(q[3][1] - q[0][1]) - 720.).abs() < 1e-8);
    }
}

#[test]
fn rotations_preserve_source_dimensions_at_unequal_scales_and_nonzero_origin() {
    let bounds = [16., 112., 12., 84.];
    let (mut capture, mut evidence, _) = fixture([128, 96], bounds, None, true);
    capture.area = [13, 29, 1280, 1920].into();
    let areas = baseline(&capture, bounds, [true; 4]);
    for pose in [-0.25_f64, 0.125, 0.375] {
        let (s, c) = pose.to_radians().sin_cos();
        for (side, &bound) in bounds.iter().enumerate() {
            let mut cue = line(
                if side < 2 {
                    (bound - s * 48.) / c
                } else {
                    (bound + s * 64.) / c
                },
                true,
            );
            cue.angle_degrees = if side < 2 { -pose } else { pose };
            evidence
                .lines
                .insert(crate::scan_regions::lines::SIDES[side].into(), vec![cue]);
        }
        let mut truth = rectangle(capture.size, bounds);
        truth.angle = pose;
        let q = truth.frame(capture.area).unwrap();
        let size = [
            (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]),
            (q[3][0] - q[0][0]).hypot(q[3][1] - q[0][1]),
        ];
        let fit = solve(
            &capture,
            size,
            observations(&target(&capture, &evidence, &areas)).unwrap(),
        );
        assert!((fit.geometry.angle - pose).abs() < 1e-10);
        assert!(fit.rms < 1e-8);
        let fitted = fit.geometry.frame(capture.area).unwrap();
        for (a, b) in fitted.iter().zip(q) {
            assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-8);
        }
    }
}

#[test]
fn differing_native_directions_have_an_interior_continuous_pose_solution() {
    let bounds = [16., 112., 12., 84.];
    let (capture, mut evidence, _) = fixture([128, 96], bounds, None, true);
    for side in 0_usize..4 {
        let physical = if side.is_multiple_of(2) { 0.25 } else { -0.25 };
        evidence
            .lines
            .get_mut(crate::scan_regions::lines::SIDES[side])
            .unwrap()[0]
            .angle_degrees = if side < 2 { -physical } else { physical };
    }
    let areas = baseline(&capture, bounds, [true; 4]);
    let fit = solve(
        &capture,
        [960., 720.],
        observations(&target(&capture, &evidence, &areas)).unwrap(),
    );
    // Opposite equal-and-opposite slope errors around matching midpoints cancel.
    assert!(fit.geometry.angle.abs() < 1e-6);
    for (got, expected) in fit.geometry.sides.into_iter().zip(bounds) {
        assert!((got.unwrap() - expected).abs() < 1e-6);
    }
}

#[test]
fn weak_missing_or_unavailable_observations_are_not_fabricated_and_gaps_remain_gaps() {
    let bounds = [16., 112., 12., 84.];
    let (capture, mut evidence, _) = fixture([128, 96], bounds, None, true);
    let complete = baseline(&capture, bounds, [true; 4]);
    let mut weak = complete.clone();
    weak.picture_weak = [true; 4];
    assert!(observations(&target(&capture, &evidence, &weak)).is_err());
    let opposite = baseline(&capture, bounds, [true, true, false, false]);
    assert!(observations(&target(&capture, &evidence, &opposite)).is_err());
    evidence.lines.get_mut("left").unwrap()[0].span_intervals = Some(vec![]);
    assert!(observations(&target(&capture, &evidence, &complete)).is_err());
    evidence.lines.get_mut("left").unwrap()[0].span_intervals = Some(vec![[0.1, 0.2], [0.8, 0.9]]);
    let data = observations(&target(&capture, &evidence, &complete)).unwrap();
    let left = data.iter().find(|v| v.side == 0).unwrap();
    assert!((left.points.iter().map(|(_, w)| w).sum::<f64>() - 1.).abs() < 1e-12);
    assert!(
        left.points
            .iter()
            .all(|(p, _)| p[1] <= 95. * 0.2 + 0.5 || p[1] >= 95. * 0.8 + 0.5)
    );
}

#[test]
fn routing_prefers_qualified_native_fit_without_running_an_unneeded_pair_search() {
    let bounds = [16., 112., 12., 84.];
    let (capture, evidence, scene) = fixture([128, 96], bounds, None, true);
    let calibration = calibration_for(&capture, bounds);
    let areas = baseline(&capture, bounds, [true; 4]);
    let outcome = evaluate_optional(
        &scene,
        target(&capture, &evidence, &areas),
        &scene.roles(&capture),
        &calibration,
        || Ok(()),
    )
    .unwrap();
    assert!(outcome.used_joint && outcome.two_edge.is_none());
    assert_eq!(outcome.proposal.as_ref().unwrap().measured, [true; 4]);
    assert_eq!(outcome.proposal.as_ref().unwrap().inferred, [false; 4]);
    content_is_preserved(
        &capture,
        outcome.proposal.as_ref().unwrap(),
        &rectangle(capture.size, bounds),
    );
}

#[test]
fn rejection_and_insufficient_observations_preserve_exact_pair_outcome() {
    let bounds = [16., 112., 12., 84.];
    for kind in ["two_sides", "blank", "film_cut", "empty_span"] {
        let (mut capture, mut evidence, scene) = fixture(
            [128, 96],
            bounds,
            if kind == "film_cut" { Some(100.) } else { None },
            true,
        );
        let mut areas = baseline(&capture, bounds, [true; 4]);
        if kind == "two_sides" {
            areas.picture_observed = [true, false, false, true];
        }
        if kind == "blank" {
            capture.rgb.fill([20000; 3]);
        }
        if kind == "empty_span" {
            evidence.lines.get_mut("left").unwrap()[0].span_intervals = Some(vec![]);
        }
        let calibration = calibration_for(&capture, bounds);
        let roles = scene.roles(&capture);
        let original = search(
            &scene,
            target(&capture, &evidence, &areas),
            &roles,
            &calibration,
            || Ok(()),
        )
        .unwrap();
        let outcome = evaluate_optional(
            &scene,
            target(&capture, &evidence, &areas),
            &roles,
            &calibration,
            || Ok(()),
        )
        .unwrap();
        assert!(!outcome.used_joint && outcome.joint_fit.error.is_some());
        // Consistency with the pre-existing route protects exact fallback, not truth.
        assert_eq!(
            serde_json::to_value(&outcome.proposal).unwrap(),
            serde_json::to_value(&original.proposal).unwrap()
        );
        assert_eq!(
            serde_json::to_value(outcome.two_edge.unwrap()).unwrap(),
            serde_json::to_value(original).unwrap()
        );
    }
}

#[test]
fn all_supplied_sides_requalify_on_completed_companions_before_joint_acceptance() {
    let bounds = [8., 44., 6., 42.];
    let (mut capture, evidence, scene) = fixture([96, 64], bounds, None, false);
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
    let areas = baseline(&capture, bounds, [true, false, true, true]);
    let roles = scene.roles(&capture);
    let mut calibration = calibration_for(&capture, bounds);
    calibration.template = calibration_for(&capture, [8., 88., 6., 42.]).template;
    let provisional = calibration.template.as_ref().unwrap().frame(0.);
    assert!(
        Mode::Joint
            .measure(
                &capture,
                &roles,
                &provisional,
                2,
                calibration.template.as_ref().unwrap().exterior[2]
            )
            .measured()
    );
    let report = evaluate(
        &scene,
        &target(&capture, &evidence, &areas),
        &roles,
        &calibration,
        || Ok(()),
    )
    .unwrap();
    assert!(report.fit.is_some() && report.proposal.is_none());
    assert!(report.failed_anchors.contains(&2) && report.failed_anchors.contains(&3));
}

#[test]
fn adaptive_joint_route_preserves_content_blank_abstention_and_nominal_cut_framing() {
    let bounds = [16., 112., 12., 84.];
    for seed in [15001, 15002, 15003, 15004, 71801, 71802, 71803, 71804] {
        for kind in ["weak", "partial", "blank", "interior"] {
            let (capture, evidence, scene, truth) = match kind {
                "interior" => known_limitations::interior_line(seed),
                "partial" => weak_fixtures::patch_surface(seed, 0.04, true, 3),
                _ => weak_fixtures::surface(seed, 0.04, true, kind == "blank"),
            };
            let areas = Areas {
                picture_geometry: Some(truth.clone()),
                picture_observed: if kind == "blank" {
                    [false; 4]
                } else {
                    [true, false, true, true]
                },
                ..Areas::default()
            };
            let calibration = calibration_for(&capture, bounds);
            let outcome = evaluate_optional(
                &scene,
                target(&capture, &evidence, &areas),
                &scene.roles(&capture),
                &calibration,
                || Ok(()),
            )
            .unwrap();
            if kind == "blank" {
                assert!(outcome.proposal.is_none());
            } else {
                content_is_preserved(&capture, &outcome.proposal.unwrap(), &truth);
            }
        }
    }
    let (capture, evidence, scene) = fixture([128, 96], bounds, Some(100.), true);
    let areas = baseline(&capture, bounds, [true; 4]);
    let choice = evaluate_optional(
        &scene,
        target(&capture, &evidence, &areas),
        &scene.roles(&capture),
        &calibration_for(&capture, bounds),
        || Ok(()),
    )
    .unwrap()
    .proposal
    .expect("known frame dimensions remain useful across missing film");
    assert!(
        !choice.measured[1] && choice.inferred[1],
        "the film cut cannot become a measured picture edge"
    );
    assert_eq!(
        choice.crop.region(capture.area).unwrap(),
        SourceRegion::from([171, 137, 960, 720])
    );
}

#[test]
fn a_mislocated_native_cue_does_not_become_a_joint_measurement_or_destroy_valid_fallback() {
    let bounds = [16., 112., 12., 84.];
    let (capture, mut evidence, scene) = fixture([128, 96], bounds, None, true);
    evidence.lines.get_mut("right").unwrap()[0].midpoint_pixel = 108.;
    let areas = baseline(&capture, [16., 108.5, 12., 84.], [true; 4]);
    let calibration = calibration_for(&capture, bounds);
    let roles = scene.roles(&capture);
    let original = search(
        &scene,
        target(&capture, &evidence, &areas),
        &roles,
        &calibration,
        || Ok(()),
    )
    .unwrap();
    assert!(original.proposal.is_some());
    let outcome = evaluate_optional(
        &scene,
        target(&capture, &evidence, &areas),
        &roles,
        &calibration,
        || Ok(()),
    )
    .unwrap();
    assert!(outcome.joint_fit.fit.is_some());
    assert!(outcome.joint_fit.proposal.is_none() && !outcome.used_joint);
    assert_eq!(
        serde_json::to_value(&outcome.proposal).unwrap(),
        serde_json::to_value(&original.proposal).unwrap()
    );
    assert_eq!(
        serde_json::to_value(outcome.two_edge.unwrap()).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    content_is_preserved(
        &capture,
        &outcome.proposal.unwrap(),
        &rectangle(capture.size, bounds),
    );
}

#[test]
fn source_clipped_side_stays_inferred_and_cancellation_does_not_fall_back() {
    let bounds = [16., 112., -8., 64.];
    let (capture, evidence, scene) = fixture([128, 96], bounds, None, true);
    let calibration = calibration_for(&capture, [16., 112., 12., 84.]);
    let areas = baseline(&capture, bounds, [true, true, false, true]);
    let outcome = evaluate_optional(
        &scene,
        target(&capture, &evidence, &areas),
        &scene.roles(&capture),
        &calibration,
        || Ok(()),
    )
    .unwrap();
    assert!(outcome.used_joint);
    let choice = outcome.proposal.unwrap();
    assert!(choice.inferred[2] && !choice.measured[2] && choice.source_clipped);
    choice.crop.region(capture.area).unwrap();
    for stop in [1, 2] {
        let calls = std::cell::Cell::new(0);
        let outcome = evaluate_optional(
            &scene,
            target(&capture, &evidence, &areas),
            &scene.roles(&capture),
            &calibration,
            || {
                calls.set(calls.get() + 1);
                if calls.get() == stop {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(outcome,Err(reason) if reason=="cancelled"));
    }
}
