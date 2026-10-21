//! Composed discovery controls pair weak-edge power with missing-edge and blank nulls.
use super::tests::{reference, template};
use super::weak_fixtures::{patch_surface, soft_surface, surface};
use super::*;

fn discover_weak(capture: &Capture, evidence: &Evidence, scene: &Scene) -> Report {
    evaluate(
        scene,
        Target {
            capture,
            evidence,
            baseline: &Areas::default(),
            lines: std::array::from_fn(|_| None),
        },
        &scene.roles(capture),
        &reference(4. / 3.),
        Some(template(capture.size, [16., 112., 12., 84.])),
        || Ok(()),
    )
    .unwrap()
}

#[test]
fn authored_weak_right_edge_still_needs_identification_before_it_can_be_measured() {
    let (capture, evidence, scene, _) = surface(100, 0., false, false);
    let report = discover_weak(&capture, &evidence, &scene);
    let choice = report
        .proposal
        .expect("the other three authored strong sides support completion");
    assert!(!choice.measured[1]);
    assert!(choice.inferred[1]);
}

#[test]
fn distributed_weak_edge_is_identified_without_a_native_right_cue() {
    for correlated in [false, true] {
        let mut identified = 0;
        for seed in 1001..1017 {
            let (capture, evidence, scene, truth) = surface(seed, 0.04, correlated, false);
            let report = discover_weak(&capture, &evidence, &scene);
            if let Some(choice) = report.proposal {
                if choice.measured[1] {
                    assert_eq!(choice.measured, [true; 4]);
                    // Two-pixel blur, +/-1.5 response kernel and half-pixel phase give
                    // a three-pixel response-support bound, not physical border precision.
                    assert!(
                        (choice.physical.sides[1].unwrap() - truth.sides[1].unwrap()).abs() <= 3.,
                        "seed {seed} correlated {correlated}: {:?}",
                        choice.physical.sides
                    );
                }
                identified += usize::from(choice.measured[1]);
            }
        }
        // Authored power at a specified SNR; not a photographic detection-rate claim.
        assert!(
            identified >= 12,
            "{identified}/16 weak edges identified; correlated={correlated}"
        );
    }
}

#[test]
fn three_strong_anchors_do_not_certify_a_fourth_from_noise_during_search() {
    for correlated in [false, true] {
        for seed in 1001..1017 {
            let (capture, evidence, scene, _) = surface(seed, 0., correlated, false);
            let report = discover_weak(&capture, &evidence, &scene);
            if let Some(choice) = report.proposal {
                assert!(
                    !choice.measured[1],
                    "zero-density seed {seed} correlated {correlated}: {:?}",
                    choice.physical.sides
                );
                assert!(choice.inferred[1]);
            }
        }
    }
}

#[test]
fn concentrating_the_same_boundary_density_in_one_short_patch_does_not_certify_a_side() {
    for correlated in [false, true] {
        let mut measured = 0;
        for seed in 1001..1017 {
            let (capture, _, scene, frame) = patch_surface(seed, 0.04, correlated, 3);
            let side = Mode::Joint.measure(
                &capture,
                &scene.roles(&capture),
                &frame,
                1,
                template(capture.size, [16., 112., 12., 84.]).exterior[1],
            );
            measured += usize::from(side.measured());
        }
        assert!(
            measured <= 1,
            "{measured}/16 concentrated cues admitted; correlated={correlated}"
        );
    }
}

#[test]
fn weak_optically_soft_edges_can_be_unresolved_without_becoming_contradictions() {
    for blur in [2., 4., 6.] {
        for seed in 1001..1017 {
            let (capture, _, scene, frame) = soft_surface(seed, blur);
            let side = Mode::Joint.measure(
                &capture,
                &scene.roles(&capture),
                &frame,
                1,
                template(capture.size, [16., 112., 12., 84.]).exterior[1],
            );
            assert!(!side.contradiction, "blur {blur} seed {seed}");
        }
    }
}

#[test]
fn correlated_and_iid_blank_targets_do_not_become_frames_during_candidate_search() {
    for correlated in [false, true] {
        for seed in 1001..1017 {
            let (capture, evidence, scene, _) = surface(seed, 0., correlated, true);
            let report = discover_weak(&capture, &evidence, &scene);
            assert!(
                report.proposal.is_none(),
                "blank seed {seed} correlated={correlated}"
            );
        }
    }
}

#[test]
fn concentrated_patches_are_not_promoted_by_adaptive_side_discovery() {
    // Four consecutive seeds cover every declared quarter-pixel phase. Repeat the
    // adaptive search at both ends and the middle, under both declared noise models.
    for patch in [0, 3, 7] {
        for correlated in [false, true] {
            for seed in 1001..1005 {
                let (capture, evidence, scene, _) = patch_surface(seed, 0.04, correlated, patch);
                if let Some(choice) = discover_weak(&capture, &evidence, &scene).proposal {
                    assert!(
                        !choice.measured[1],
                        "patch {patch} seed {seed} correlated {correlated}: {:?}",
                        choice.physical.sides
                    );
                    assert!(choice.inferred[1]);
                }
            }
        }
    }
}
