//! Reproducible adverse evidence, not assertions that freeze incorrect detector behavior.
use super::tests::{reference, template};
use super::weak_fixtures::surface;
use super::*;

// Reuses the consumed stronger-interior-line construction. Density is applied through
// phased physical coverage; integer-box rendering would invent a different ground truth.
pub(super) fn interior_line(seed: u64) -> (Capture, Evidence, Scene, PictureGeometry) {
    let (mut capture, evidence, scene, frame) = surface(seed, 0.08, false, false);
    let line = frame.sides[1].unwrap() - 3.;
    for y in 0..96 {
        for x in 0..128 {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let bounds = frame.sides.map(Option::unwrap);
            let distance = (px - bounds[0])
                .min(bounds[1] - px)
                .min(py - bounds[2])
                .min(bounds[3] - py);
            let coverage = (distance / 2. + 0.5).clamp(0., 1.);
            let density = ((line - px) / 0.5 + 0.5).clamp(0., 1.) * 0.8 * coverage;
            for value in &mut capture.rgb[y * 128 + x] {
                *value = (f64::from(*value) * (-density).exp2()).round() as u16;
            }
        }
    }
    (capture, evidence, scene, frame)
}

#[test]
#[ignore = "Known Joint03 scene-line association failure; opt-in physical acceptance check, not a passing baseline guarantee"]
fn a_stronger_interior_line_must_not_replace_the_physical_picture_border() {
    let seed = 15001;
    let (capture, evidence, scene, truth) = interior_line(seed);
    let report = evaluate(
        &scene,
        Target {
            capture: &capture,
            evidence: &evidence,
            baseline: &Areas::default(),
            lines: [None, None, None, None],
        },
        &scene.roles(&capture),
        &reference(4. / 3.),
        Some(template(capture.size, [16., 112., 12., 84.])),
        || Ok(()),
    )
    .unwrap();
    // Abstention is safe here; a returned crop must not substitute the scene line.
    if let Some(choice) = report.proposal {
        // Authored two-pixel transition plus half-pixel source phase: this tolerance
        // includes its fuzzy edge but excludes the line three pixels inside it.
        assert!(
            (choice.physical.sides[1].unwrap() - truth.sides[1].unwrap()).abs() <= 1.5,
            "known association failure seed {seed}: true right {:?}, selected {:?}, measured {:?}",
            truth.sides[1],
            choice.physical.sides,
            choice.measured
        );
    }
}
