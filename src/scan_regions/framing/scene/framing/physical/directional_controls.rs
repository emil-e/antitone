use super::super::tests::fixture;
use super::tests::{geometry, reference, template};
use super::*;

#[test]
fn source_censoring_and_known_film_cut_do_not_turn_into_directional_anchors() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., -3., 42.], None, false);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &geometry(capture.size, [8., 56., -3., 42.]),
        2,
        template(capture.size, [8., 56., -3., 42.]).exterior[2],
    );
    assert!(!side.measured());
    assert!(!side.contradiction);
    let (capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], Some(50.), false);
    let result = evaluate_with(
        &scene,
        Target {
            capture: &capture,
            evidence: &evidence,
            baseline: &Areas::default(),
            lines: [None, None, None, None],
        },
        &scene.roles(&capture),
        &reference(4. / 3.),
        Some(template(capture.size, [8., 56., 6., 42.])),
        || Ok(()),
        Mode::Joint,
    )
    .unwrap();
    assert!(result.proposal.is_none());
}

#[test]
fn real_separated_fragments_are_not_photo_continuation_on_either_axis() {
    for side in [0, 2] {
        let (mut capture, _, scene) = fixture([72, 72], [20., 52., 20., 52.], None, false);
        for y in 0..72 {
            for x in 0..72 {
                if (side == 0 && x < 16 && (20..52).contains(&y))
                    || (side == 2 && y < 16 && (20..52).contains(&x))
                {
                    capture.rgb[y * 72 + x] = [8000; 3];
                }
            }
        }
        let frame = geometry(capture.size, [20., 52., 20., 52.]);
        let result = directional::measure(
            &capture,
            &scene.roles(&capture),
            &frame,
            side,
            template(capture.size, [20., 52., 20., 52.]).exterior[side],
        );
        assert!(result.measured());
        assert!(!result.contradiction);
        assert!(result.neighbor.is_some());
    }
}

#[test]
fn a_narrow_observed_rebate_can_end_in_known_holder_without_a_photo_veto() {
    let (mut capture, _, mut scene) = fixture([64, 48], [12., 60., 6., 42.], None, false);
    for y in 0..48 {
        for x in 0..11 {
            capture.rgb[y * 64 + x] = [100; 3];
            scene.holder[y * 64 + x] = true;
        }
    }
    let frame = geometry(capture.size, [12., 60., 6., 42.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        0,
        template(capture.size, [12., 60., 6., 42.]).exterior[0],
    );
    assert!(side.measured());
    assert!(!side.contradiction);
}

#[test]
fn directional_evidence_keeps_arbitrary_learned_hues_and_blank_target_abstention() {
    for base in color_tests::BASES {
        for blank in [false, true] {
            let (captures, scene, prior) = color_tests::coloured_roll(base, blank);
            let capture = &captures[3];
            let (_, evidence, _) = fixture(capture.size, [8., 56., 6., 42.], None, true);
            let baseline = if blank {
                Areas::default()
            } else {
                Areas {
                    picture_geometry: Some(geometry(capture.size, [8., 56., 6., 42.])),
                    picture_observed: [true; 4],
                    ..Areas::default()
                }
            };
            let report = evaluate_with(
                &scene,
                Target {
                    capture,
                    evidence: &evidence,
                    baseline: &baseline,
                    lines: [None, None, None, None],
                },
                &scene.roles(capture),
                &reference(4. / 3.),
                Some(prior),
                || Ok(()),
                Mode::Joint,
            )
            .unwrap();
            assert_eq!(report.proposal.is_none(), blank);
            if let Some(choice) = report.proposal {
                assert_eq!(choice.measured, [true; 4]);
            }
        }
    }
}

#[test]
fn noise_only_does_not_systematically_produce_distributed_boundary_evidence() {
    let mut false_edges = 0;
    for seed in 1..=64 {
        let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
        let mut state = seed as u64;
        for pixel in &mut capture.rgb {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = 20000. + ((state >> 32) as f64 / u32::MAX as f64 - 0.5) * 1800.;
            *pixel = [value.round() as u16; 3];
        }
        let roles = scene.roles(&capture);
        let frame = geometry(capture.size, [16., 112., 12., 84.]);
        for side in 0..4 {
            let measured = directional::measure(
                &capture,
                &roles,
                &frame,
                side,
                template(capture.size, [16., 112., 12., 84.]).exterior[side],
            );
            if measured.measured() {
                let report = serde_json::to_value(&measured.directional).unwrap();
                eprintln!(
                    "null seed={seed} side={side} old={} aggregate={} blocks={}",
                    report["old_measured"], report["aggregate_z"], report["blocks"]
                );
                false_edges += 1;
            }
        }
    }
    // A bounded authored null-regression budget, not a calibrated photographic error rate.
    assert!(
        false_edges <= 2,
        "{false_edges}/256noise-onlyedges admitted"
    );
}
