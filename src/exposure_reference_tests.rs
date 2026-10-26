use super::*;

#[test]
fn normalized_ap1_precedes_repairs_and_is_not_normalized_again_by_grading() {
    use crate::adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerPipeline, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
    };
    use crate::processing::{InputPipeline, InputSettings, InvertedPixel, RollSettings};
    use glam::{DMat3, DVec3};
    let input = InputSettings::Digital;
    let physical = DMat3::from_diagonal(DVec3::new(0.8, 1.2, 0.7));
    let camera = physical.inverse() * DVec3::splat(0.045);
    let matrix = physical * 4.;
    let ungraded = InputPipeline::ungraded(input, matrix)
        .unwrap()
        .apply(camera)
        .unwrap();
    assert!((ungraded.rgb - DVec3::splat(0.18)).abs().max_element() < 1e-14);
    let pipeline = AdjustmentLayerPipeline::new(
        input,
        RollSettings::default(),
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 1.5,
            pivot: 0.18,
            ..Default::default()
        })])
        .unwrap(),
        matrix,
    )
    .unwrap();
    let expected = DVec3::splat(0.18);
    assert!(
        (pipeline.apply(camera, [0, 0]).unwrap().rgb - expected)
            .abs()
            .max_element()
            < 1e-12
    );
    assert!(
        (pipeline
            .grade_neutral(
                InvertedPixel {
                    rgb: ungraded.rgb,
                    floored: [false; 3]
                },
                [0, 0]
            )
            .unwrap()
            .rgb
            - expected)
            .abs()
            .max_element()
            < 1e-12
    );
    let manual = AdjustmentLayerPipeline::new(
        input,
        RollSettings {
            exposure: 1.,
            ..Default::default()
        },
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: -0.5,
            ..Default::default()
        })])
        .unwrap(),
        matrix,
    )
    .unwrap();
    assert!(
        (manual.apply(camera, [0, 0]).unwrap().rgb - DVec3::splat(0.045 * 2_f64.powf(2.5)))
            .abs()
            .max_element()
            < 1e-12
    );
}

#[test]
fn neutral_display_reference_includes_the_aces_tone_response() {
    // Pinned 100-nit ACES maps scene 0.18 to display-linear 0.1, not 0.18.
    let matched = estimate(std::iter::repeat_n((0.18, 0.1), 256)).unwrap();
    assert!(matched.ev.abs() < 0.005, "{matched:?}");
    let brighter = estimate(std::iter::repeat_n((0.18, 0.18), 256)).unwrap();
    assert!((brighter.ev - 0.7768156).abs() < 0.005, "{brighter:?}");
}

#[test]
fn no_reference_is_distinct_from_a_valid_zero_adjustment() {
    assert!(estimate([]).is_none());
    assert!(estimate(std::iter::repeat_n((0.18, 0.1), 127)).is_none());
    for sample in [
        (0., 0.1),
        (-0.1, 0.1),
        (f64::NAN, 0.1),
        (0.18, 0.),
        (0.18, 1.),
        (0.18, f64::INFINITY),
    ] {
        assert!(estimate(std::iter::repeat_n(sample, 256)).is_none());
    }
    assert!(estimate(std::iter::repeat_n((0.18, 0.1), 128)).is_some());
}

#[cfg(feature = "ocio-reference")]
#[test]
fn quantile_fit_recovers_known_offsets_and_retains_paired_capture_differences() {
    use crate::rendering::{CpuRenderer, OutputTarget};
    use glam::DVec3;
    let renderer = CpuRenderer::new(OutputTarget::Srgb).unwrap();
    for desired in [-0.7_f64, 0.6, 1.3] {
        for capture in [-1_f64, 0., 1.] {
            let source = (0..1024)
                .map(|i| 0.03 * (1. + i as f64 / 100.) * capture.exp2())
                .collect::<Vec<_>>();
            let rendered = renderer
                .render(
                    &source
                        .iter()
                        .map(|v| DVec3::splat(*v * desired.exp2()))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let samples = source.iter().zip(rendered).map(|(a, b)| {
                // Model an 8-bit preview; estimator input is linear after decoding.
                let encoded =
                    (OutputTarget::Srgb.encode(b.y as f64).unwrap() * 255.).round() / 255.;
                (*a, OutputTarget::Srgb.decode(encoded).unwrap())
            });
            let result = estimate(samples).unwrap();
            assert!(
                (result.ev - desired).abs() < 0.025,
                "desired={desired} capture={capture} {result:?}"
            );
        }
    }
}

#[cfg(feature = "ocio-reference")]
#[test]
fn generated_inverse_table_tracks_the_reference_over_the_accepted_range() {
    use crate::rendering::{CpuRenderer, OutputTarget};
    use glam::DVec3;
    let renderer = CpuRenderer::new(OutputTarget::Srgb).unwrap();
    for i in 0..1025 {
        let output = 0.015 + (0.8 - 0.015) * i as f64 / 1024.;
        let scene = inverse_neutral(output).exp2();
        let actual = renderer.render(&[DVec3::splat(scene)]).unwrap()[0].y as f64;
        assert!(
            (actual / output).log2().abs() < 0.005,
            "{output} => {scene} => {actual}"
        );
    }
}

#[test]
fn normalization_scale_does_not_make_well_conditioned_source_transforms_singular() {
    use crate::processing::{InputPipeline, InputSettings, validate_camera_matrix};
    use glam::{DMat3, DVec3};
    for gain in [2_f64.powi(-16), 2_f64.powi(-14), 1., 2_f64.powi(16)] {
        let matrix = DMat3::IDENTITY * gain;
        validate_camera_matrix(matrix).unwrap();
        let pipeline = InputPipeline::ungraded(InputSettings::Digital, matrix).unwrap();
        let expected = DVec3::splat(0.18 * gain);
        assert!(
            (pipeline.apply(DVec3::splat(0.18)).unwrap().rgb - expected)
                .abs()
                .max_element()
                < expected.max_element() * 1e-12
        );
        let singular = DMat3::from_cols(DVec3::X, DVec3::X, DVec3::Z) * gain;
        assert!(validate_camera_matrix(singular).is_err());
    }
    assert!(validate_camera_matrix(DMat3::ZERO).is_err());
    assert!(
        validate_camera_matrix(DMat3::from_cols(
            DVec3::X,
            DVec3::X + DVec3::Y * 1e-14,
            DVec3::Z
        ))
        .is_err()
    );
    validate_camera_matrix(DMat3::from_diagonal(DVec3::new(
        2_f64.powi(-80),
        2_f64.powi(40),
        2_f64.powi(40),
    )))
    .unwrap();
}
