use super::super::tests::fixture;
use super::tests::{geometry, reference, template};
use super::*;

fn surface(width: f64, dim: bool) -> (Capture, Scene) {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, true);
    let mut seed = 71u64;
    for y in 0..96 {
        for x in 0..128 {
            let inside = (x as f64 + 0.5 - 16.)
                .min(112. - (x as f64 + 0.5))
                .min(y as f64 + 0.5 - 12.)
                .min(84. - (y as f64 + 0.5));
            let alpha = if width == 0. {
                f64::from(inside >= 0.)
            } else {
                (inside / width + 0.5).clamp(0., 1.)
            };
            let band = ((y as f64 - 12.).max(0.) / 72. * 8.).floor() as usize;
            let transmission = match band % 8 {
                1 | 4 | 7 => 1.,
                0 => 0.7,
                2 => 0.4,
                3 => 0.85,
                5 => 0.3,
                _ => 0.6,
            };
            let base = if dim { 16000. } else { 20000. };
            let rgb = std::array::from_fn(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let noise = ((seed >> 32) as f64 / u32::MAX as f64 - 0.5) * 40.;
                (base * (1. - alpha * (1. - transmission)) + noise)
                    .round()
                    .clamp(2., 65534.) as u16
            });
            capture.rgb[y * 128 + x] = rgb;
        }
    }
    (capture, scene)
}

#[test]
fn heterogeneous_density_and_quiet_sections_support_a_common_blurred_boundary() {
    for width in [0., 2., 4.] {
        let (capture, scene) = surface(width, false);
        let frame = geometry(capture.size, [16., 112., 12., 84.]);
        let side = directional::measure(
            &capture,
            &scene.roles(&capture),
            &frame,
            1,
            template(capture.size, [16., 112., 12., 84.]).exterior[1],
        );
        assert!(side.measured(), "blur width{width}");
        assert!(!side.contradiction);
    }
}

#[test]
fn dim_compatible_rebate_is_not_relabelled_photo_only_from_absolute_colour() {
    let (capture, scene) = surface(2., true);
    let frame = geometry(capture.size, [16., 112., 12., 84.]);
    let exterior = template(capture.size, [16., 112., 12., 84.]).exterior[1];
    assert!(
        perimeter::measure(&capture, &scene.roles(&capture), &frame, 1, exterior).contradiction
    );
    let side = directional::measure(&capture, &scene.roles(&capture), &frame, 1, exterior);
    assert!(side.measured());
    assert!(!side.contradiction);
}

#[test]
fn one_extreme_short_segment_cannot_outvote_photo_continuing_along_the_side() {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 12..20 {
        for x in 96..112 {
            capture.rgb[y * 128 + x] = [20000; 3];
        }
    }
    let frame = geometry(capture.size, [16., 96., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.measured());
    assert!(side.contradiction);
}

#[test]
fn interleaved_photographic_continuation_cannot_hide_between_supporting_bins() {
    for phase in 0..9 {
        let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
        for y in 12..84 {
            if ((y - 12 + phase) / 9) % 2 == 0 {
                for x in 96..112 {
                    capture.rgb[y * 128 + x] = [20000; 3];
                }
            }
        }
        let frame = geometry(capture.size, [16., 96., 12., 84.]);
        let side = directional::measure(
            &capture,
            &scene.roles(&capture),
            &frame,
            1,
            template(capture.size, [16., 112., 12., 84.]).exterior[1],
        );
        assert!(side.contradiction, "phase {phase}");
        assert!(!side.measured(), "phase {phase}");
    }
}

#[test]
fn a_coherent_signed_scene_edge_does_not_erase_structured_photographic_exterior() {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 12..84 {
        for x in 16..112 {
            let texture = match (y / 4) % 4 {
                0 => 0.65,
                1 => 1.2,
                2 => 0.8,
                _ => 1.4,
            };
            capture.rgb[y * 128 + x] = [if x < 96 {
                (4000. * texture) as u16
            } else {
                (8000. * texture) as u16
            }; 3];
        }
    }
    let frame = geometry(capture.size, [16., 96., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.measured());
    assert!(side.contradiction);
}

#[test]
fn a_smooth_illumination_ramp_is_not_a_localized_frame_edge() {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 0..96 {
        for x in 0..128 {
            capture.rgb[y * 128 + x] =
                [(20000. * (0.002 * x as f64 + 0.001 * y as f64 - 1.).exp2()).round() as u16; 3];
        }
    }
    let frame = geometry(capture.size, [16., 112., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.measured());
}

#[test]
fn a_negative_illumination_ramp_is_uncertain_not_a_reversed_transition() {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 0..96 {
        for x in 0..128 {
            capture.rgb[y * 128 + x] =
                [(20000. * (-0.002 * x as f64 + 0.001 * y as f64 - 1.).exp2()).round() as u16; 3];
        }
    }
    let frame = geometry(capture.size, [16., 112., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.measured());
    assert!(!side.contradiction);
}

#[test]
fn point_raster_phase_does_not_turn_smooth_illumination_into_exterior_texture() {
    for height in [72., 80., 100.] {
        for angle in [-4., 0., 4.] {
            for slope in [-0.002, 0.002] {
                let (mut capture, _, scene) =
                    fixture([128, 128], [16., 112., 12., 12. + height], None, false);
                for y in 0..128 {
                    for x in 0..128 {
                        capture.rgb[y * 128 + x] = [(20000.
                            * (slope * x as f64 + 0.001 * y as f64 - 1.).exp2())
                        .round() as u16; 3];
                    }
                }
                let mut frame = geometry(capture.size, [16., 112., 12., 12. + height]);
                frame.angle = angle;
                let side = directional::measure(
                    &capture,
                    &scene.roles(&capture),
                    &frame,
                    1,
                    template(capture.size, [16., 112., 12., 12. + height]).exterior[1],
                );
                assert!(
                    !side.measured(),
                    "height {height} angle {angle} slope {slope}"
                );
                assert!(
                    !side.contradiction,
                    "height {height} angle {angle} slope {slope}"
                );
            }
        }
    }
}

#[test]
fn a_localized_reversed_transition_remains_contradictory() {
    let (mut capture, _, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 0..96 {
        for x in 0..128 {
            capture.rgb[y * 128 + x] = [if x < 112 { 20000 } else { 8000 }; 3];
        }
    }
    let frame = geometry(capture.size, [16., 112., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(side.contradiction);
    assert!(!side.measured());
}

#[test]
fn a_positive_but_mislocalized_transition_is_uncertain_not_a_reversed_edge() {
    let (mut capture, scene) = surface(8., false);
    for y in 0..96 {
        for x in 0..128 {
            let coverage = 1. / (1. + ((x as f64 + 0.5 - 112.) / 0.75).exp());
            capture.rgb[y * 128 + x] = [(20000. * (1. - 0.6 * coverage)).round() as u16; 3];
        }
    }
    let frame = geometry(capture.size, [16., 115., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.contradiction);
    assert!(!side.measured());
}

#[test]
fn texture_beyond_an_observed_base_separator_is_not_target_picture_continuation() {
    let (mut capture, _, mut scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    for y in 0..96 {
        capture.rgb[y * 128 + 114] = [6000; 3];
        capture.rgb[y * 128 + 115] = [20000; 3];
        capture.rgb[y * 128 + 116] = [if (y / 4) % 2 == 0 { 3000 } else { 11000 }; 3];
        for x in 117..128 {
            capture.rgb[y * 128 + x] = [100; 3];
            scene.holder[y * 128 + x] = true;
        }
    }
    let frame = geometry(capture.size, [16., 112., 12., 84.]);
    let side = directional::measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        1,
        template(capture.size, [16., 112., 12., 84.]).exterior[1],
    );
    assert!(!side.contradiction);
    assert!(side.measured());
}

#[test]
fn a_uniform_base_coloured_target_cannot_become_a_frame_from_roll_geometry() {
    let (mut capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, true);
    capture.rgb.fill([20000; 3]);
    let report = evaluate_with(
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
    assert!(report.proposal.is_none());
}

#[test]
fn source_clipping_still_requires_three_identified_sides_and_marks_the_missing_one_inferred() {
    let (capture, evidence, scene) = fixture([64, 48], [8., 56., -3., 42.], None, false);
    let mut prior = template(capture.size, [8., 56., 1., 46.]);
    prior.sides[2].radius = 6.;
    prior.sides[3].radius = 6.;
    prior.centers[1].radius = 6.;
    let report = evaluate_with(
        &scene,
        Target {
            capture: &capture,
            evidence: &evidence,
            baseline: &Areas::default(),
            lines: [None, None, None, None],
        },
        &scene.roles(&capture),
        &reference(48. / 45.),
        Some(prior),
        || Ok(()),
        Mode::Joint,
    )
    .unwrap();
    let choice = report.proposal.unwrap();
    assert_eq!(choice.inferred, [false, false, true, false]);
    assert_eq!(choice.measured, [true, true, false, true]);
    assert!(choice.source_clipped);
}

#[test]
fn source_censoring_does_not_allow_two_anchors_to_complete_two_missing_sides() {
    let (mut capture, evidence, scene) = fixture([64, 48], [8., 56., -3., 42.], None, false);
    for y in 0..42 {
        for x in 48..64 {
            capture.rgb[y * 64 + x] = [8000; 3];
        }
    }
    let mut prior = template(capture.size, [8., 56., 1., 46.]);
    prior.sides[2].radius = 6.;
    prior.sides[3].radius = 6.;
    prior.centers[1].radius = 6.;
    let report = evaluate_with(
        &scene,
        Target {
            capture: &capture,
            evidence: &evidence,
            baseline: &Areas::default(),
            lines: [None, None, None, None],
        },
        &scene.roles(&capture),
        &reference(48. / 45.),
        Some(prior),
        || Ok(()),
        Mode::Joint,
    )
    .unwrap();
    assert!(report.proposal.is_none());
}
