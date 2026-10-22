//! A cue qualified on provisional companions is not a measurement of a completed frame.
use super::super::tests::{fixture, shape_reference};
use super::tests::{geometry, template};
use super::*;

#[test]
fn transitions_beyond_completed_companions_cannot_certify_the_target_sides() {
    let bounds = [8., 44., 6., 42.];
    let (mut capture, mut evidence, scene) = fixture([96, 64], bounds, None, false);
    capture.rgb.fill([20000; 3]);
    for y in 12..36 {
        for x in 8..12 {
            capture.rgb[y * 96 + x] = [6000; 3];
        }
    }
    for y in 6..42 {
        // Strong top/bottom cues lie outside the completed target's horizontal span.
        // Continuing to the camera limit avoids supplying a second complete rectangle.
        for x in 48..96 {
            capture.rgb[y * 96 + x] = [6000; 3];
        }
    }
    evidence.lines.get_mut("right").unwrap().clear();
    let roles = scene.roles(&capture);
    let completed = geometry(capture.size, bounds);
    let mut prior = template(capture.size, [8., 88., 6., 42.]);
    // Both the wide provisional span and narrow ratio completion fit this declared prior.
    prior.width.radius = 44.;
    prior.centers[0].radius = 22.;
    assert!(prior.compatible(&completed));
    let provisional = prior.frame(0.);
    for side in [2, 3] {
        assert!(
            Mode::Joint
                .measure(&capture, &roles, &provisional, side, prior.exterior[side])
                .measured(),
            "side {side} must qualify before completion"
        );
        assert!(
            !Mode::Joint
                .measure(&capture, &roles, &completed, side, prior.exterior[side])
                .measured(),
            "side {side} loses its evidence inside completed companions"
        );
    }
    let reference = shape_reference(Some(1.));
    let baseline = Areas::default();
    let target = || Target {
        capture: &capture,
        evidence: &evidence,
        baseline: &baseline,
        lines: [None, None, None, None],
    };
    let locations = ranked_locations(&capture, &roles, &target(), &prior, 0., Mode::Joint);
    let (phases, _) =
        joint::hypotheses(&locations, &prior, &reference, capture.area, 0., || Ok(())).unwrap();
    assert!(
        phases[1]
            .iter()
            .any(|candidate| candidate.frame.sides == completed.sides
                && candidate.proposed == [true, false, true, true]),
        "the misleading completion must reach perimeter qualification, not disappear in discovery/pruning"
    );
    let report = evaluate(&scene, target(), &roles, &reference, Some(prior), || Ok(())).unwrap();
    assert!(report.proposal.is_none());
}
