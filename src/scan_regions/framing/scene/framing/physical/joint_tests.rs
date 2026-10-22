use super::super::tests::fixture;
use super::tests::{geometry, reference, template};
use super::*;

fn discover(
    capture: &Capture,
    evidence: &Evidence,
    scene: &Scene,
    prior: Template,
    mode: Mode,
) -> Report {
    evaluate_with(
        scene,
        Target {
            capture,
            evidence,
            baseline: &Areas::default(),
            lines: std::array::from_fn(|_| None),
        },
        &scene.roles(capture),
        &reference(4. / 3.),
        Some(prior),
        || Ok(()),
        mode,
    )
    .unwrap()
}

#[test]
fn composed_discovery_does_not_censor_a_frame_allowed_by_joint_placement_and_size() {
    let bounds = [22., 118., 12., 84.];
    let (capture, evidence, scene) = fixture([128, 96], bounds, None, false);
    let mut prior = template(capture.size, [16., 112., 12., 84.]);
    // A marginal side fit can be narrower than the placement/extent model actually admitted.
    // The true frame changes placement, not physical width, height or source orientation.
    prior.centers[0].radius = 8.;
    assert!(prior.compatible(&geometry(capture.size, bounds)));
    let report = discover(&capture, &evidence, &scene, prior, Mode::Joint);
    let choice = report
        .proposal
        .expect("joint-compatible target must reach full evaluation");
    assert_eq!(choice.measured, [true; 4]);
    assert_eq!(choice.inferred, [false; 4]);
    // This is discovery/association coverage, not a subpixel border convention.
    let center = (choice.physical.sides[0].unwrap() + choice.physical.sides[1].unwrap()) * 0.5;
    assert!(
        (center - 70.).abs() < (center - 64.).abs(),
        "must associate with the authored target centre70, not the prior64"
    );
}

#[test]
fn prior_companions_do_not_hide_anchors_on_a_joint_compatible_displaced_frame() {
    for omitted in [
        vec![],
        vec!["left", "right"],
        vec!["left", "right", "top", "bottom"],
    ] {
        let (capture, mut evidence, scene) = fixture([128, 96], [48., 80., 60., 84.], None, false);
        for side in omitted {
            evidence.lines.get_mut(side).unwrap().clear();
        }
        let mut prior = template(capture.size, [48., 80., 12., 36.]);
        prior.centers[1].radius = 48.;
        assert!(prior.compatible(&geometry(capture.size, [48., 80., 60., 84.])));
        let report = discover(&capture, &evidence, &scene, prior, Mode::Joint);
        let choice = report.proposal.expect(
            "prior blank span cannot definitively reject a side visible on completed target",
        );
        assert_eq!(choice.measured, [true; 4]);
        let center = (choice.physical.sides[2].unwrap() + choice.physical.sides[3].unwrap()) * 0.5;
        assert!((center - 72.).abs() < (center - 24.).abs());
    }
}

#[test]
fn side_envelope_contains_joint_compatibility_corners_at_pose_and_unequal_source_scale() {
    for shape in [[16., 112., 12., 84.], [32., 80., 8., 88.]] {
        let mut prior = template([128, 96], shape);
        prior.pose.center = 0.2;
        prior.pose.radius = 0.7;
        prior.scale = [2., 3.];
        prior.width.center *= 2.;
        prior.width.radius *= 2.;
        prior.height.center *= 3.;
        prior.height.radius *= 3.;
        prior.centers[0].radius = 7.;
        prior.centers[1].radius = 5.;
        for angle in [-0.5, 0.2, 0.9] {
            for sx in [-1., 1.] {
                for sy in [-1., 1.] {
                    for sw in [-1., 1.] {
                        for sh in [-1., 1.] {
                            let mut corner = prior.clone();
                            corner.centers[0].center += sx * prior.centers[0].radius;
                            corner.centers[1].center += sy * prior.centers[1].radius;
                            corner.width.center += sw * prior.width.radius;
                            corner.height.center += sh * prior.height.radius;
                            let frame = corner.frame(angle);
                            for side in 0..4 {
                                let band = joint::search_band(&prior, angle, side);
                                assert!(
                                    (frame.sides[side].unwrap() - band.center).abs()
                                        <= band.radius + 1e-9,
                                    "side {side} pose {angle}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn locally_weaker_sides_are_not_discarded_before_joint_compatibility() {
    let prior = template([128, 96], [16., 112., 12., 84.]);
    let locations = [
        vec![(11.5, 2.), (12.5, 1.9), (16.5, 0.5)],
        vec![(116.5, 2.), (115.5, 1.9), (112.5, 0.5)],
        vec![(7.5, 2.), (8.5, 1.9), (12.5, 0.5)],
        vec![(88.5, 2.), (87.5, 1.9), (84.5, 0.5)],
    ];
    let (phases, _) = joint::hypotheses(
        &locations,
        &prior,
        &reference(2.7),
        [11, 17, 1280, 960].into(),
        0.,
        || Ok(()),
    )
    .unwrap();
    assert!(phases[0].iter().any(
        |candidate| candidate.frame.sides == [Some(16.5), Some(112.5), Some(12.5), Some(84.5)]
    ));
}

#[test]
fn zero_pose_envelope_uses_raster_centres_and_physical_extent_units() {
    let mut prior = template([128, 96], [16., 112., 12., 84.]);
    prior.centers[0].radius = 7.;
    prior.centers[1].radius = 5.;
    prior.scale = [2., 3.];
    prior.width.center = 192.;
    prior.width.radius = 8.;
    prior.height.center = 216.;
    prior.height.radius = 12.;
    for (side, (center, radius)) in [(16., 9.), (112., 9.), (12., 7.), (84., 7.)]
        .into_iter()
        .enumerate()
    {
        let band = joint::search_band(&prior, 0., side);
        assert!((band.center - center).abs() < 1e-9);
        assert!((band.radius - radius).abs() < 1e-9);
    }
}

#[test]
fn many_compatible_combinations_have_bounded_shortlists_and_a_deadline_checkpoint() {
    let prior = template([128, 96], [16., 112., 12., 84.]);
    let locations = std::array::from_fn(|side| {
        (0..16)
            .map(|index| {
                (
                    prior.sides[side].center - 2. + index as f64 * 0.25,
                    index as f64 / 16.,
                )
            })
            .collect()
    });
    let (phases, _) = joint::hypotheses(
        &locations,
        &prior,
        &reference(4. / 3.),
        [11, 17, 1280, 960].into(),
        0.,
        || Ok(()),
    )
    .unwrap();
    // This fixture supplies more compatible hypotheses than the configured work budget.
    // Fill that budget without silently pruning candidates or hard-coding its numeric value.
    assert!(phases.iter().all(|phase| phase.len() == joint::LIMIT));
    let stopped = joint::hypotheses(
        &locations,
        &prior,
        &reference(4. / 3.),
        [11, 17, 1280, 960].into(),
        0.,
        || Err("owned deadline".into()),
    );
    assert!(matches!(stopped,Err(message) if message=="owned deadline"));
}

#[test]
fn independent_side_envelopes_do_not_relax_complete_frame_identity() {
    let (mut capture, evidence, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    let prior = template(capture.size, [16., 112., 12., 84.]);
    let oversized = geometry(capture.size, [11., 117., 7., 89.]);
    assert!(!prior.compatible(&oversized));
    for y in 0..96 {
        for x in 0..128 {
            capture.rgb[y * 128 + x] = [if (11..117).contains(&x) && (7..89).contains(&y) {
                6000
            } else {
                20000
            }; 3];
        }
    }
    let report = discover(&capture, &evidence, &scene, prior, Mode::Joint);
    assert!(report.proposal.is_none());
}
