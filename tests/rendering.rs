#![cfg(feature = "render")]

use antitone::{
    Error,
    color::BRADFORD,
    profiles,
    rendering::{CpuRenderer, OutputTarget, ocio_version},
};
use glam::{DMat3, DVec3, Vec3};
use lcms2::{Flags, Intent, PixelFormat, Profile, Tag, TagSignature, Transform};

#[test]
fn output_transfers_have_the_correct_toes_and_reject_invalid_channels() {
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        assert!((target.encode(0.0031308).unwrap() - 0.040449936).abs() < 1e-12);
        assert!((target.decode(0.04045).unwrap() - 0.04045 / 12.92).abs() < 1e-12);
        assert_eq!(target.encode(0.0).unwrap(), 0.0);
        assert!((target.encode(1.0).unwrap() - 1.0).abs() < 1e-15);
        for i in 0..=65535 {
            let code = f64::from(i) / 65535.0;
            let linear = target.decode(code).unwrap();
            assert!((target.encode(linear).unwrap() - code).abs() < 3e-8);
            let quantized = target.encode_u16(&[Vec3::splat(linear as f32)]).unwrap()[0];
            assert_eq!(quantized, [i as u16; 3]);
        }
        for invalid in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            assert!(target.encode(invalid).is_err());
            assert!(target.decode(invalid).is_err());
        }
    }
}

#[test]
fn serialized_icc_trcs_match_each_encoded_16_bit_value() {
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        for bytes in [
            profiles::icc(target).to_vec(),
            profiles::generate(target).unwrap(),
        ] {
            let profile = Profile::new_icc(&bytes).unwrap();
            assert!(profile.is_matrix_shaper());
            for tag in [
                TagSignature::RedTRCTag,
                TagSignature::GreenTRCTag,
                TagSignature::BlueTRCTag,
            ] {
                let Tag::ToneCurve(curve) = profile.read_tag(tag) else {
                    panic!("missing TRC")
                };
                assert_eq!(curve.parametric_type(), 4);
                let mut previous = -1.0;
                for i in 0..=65535 {
                    let encoded = f64::from(i) / 65535.0;
                    let actual = f64::from(curve.eval(encoded as f32));
                    let expected = target.decode(encoded).unwrap();
                    assert!(actual >= previous, "nonmonotonic TRC at {i}");
                    assert!(
                        (actual - expected).abs() < 5e-6,
                        "{target:?} {i}: {actual} vs {expected}"
                    );
                    previous = actual;
                }
            }
        }
    }
}

fn target_to_pcs(target: OutputTarget) -> DMat3 {
    let p = target.parameters();
    let xyz = |x: f64, y: f64| DVec3::new(x / y, 1.0, (1.0 - x - y) / y);
    let white = xyz(p[7], p[8]);
    let primaries = DMat3::from_cols(xyz(p[1], p[2]), xyz(p[3], p[4]), xyz(p[5], p[6]));
    let matrix = primaries * DMat3::from_diagonal(primaries.inverse() * white);
    let pcs_white = DVec3::new(0.9642, 1.0, 0.8249);
    let adaptation = BRADFORD.inverse()
        * DMat3::from_diagonal((BRADFORD * pcs_white) / (BRADFORD * white))
        * BRADFORD;
    adaptation * matrix
}

#[test]
fn serialized_icc_primaries_and_float_conversion_match_the_rendering_target() {
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let profile = Profile::new_icc(profiles::icc(target)).unwrap();
        let expected = target_to_pcs(target);
        for (index, tag) in [
            TagSignature::RedColorantTag,
            TagSignature::GreenColorantTag,
            TagSignature::BlueColorantTag,
        ]
        .into_iter()
        .enumerate()
        {
            let Tag::CIEXYZ(value) = profile.read_tag(tag) else {
                panic!("missing colorant")
            };
            let actual = DVec3::new(value.X, value.Y, value.Z);
            assert!((actual - expected.col(index)).abs().max_element() < 2e-5);
        }
        let mut linear = Vec::new();
        let mut encoded = Vec::new();
        for r in [0.0, 0.0001, 1.0 / 512.0, 0.18, 1.0] {
            for g in [0.0, 0.0001, 1.0 / 512.0, 0.18, 1.0] {
                for b in [0.0, 0.0001, 1.0 / 512.0, 0.18, 1.0] {
                    let pixel = DVec3::new(r, g, b);
                    linear.push(pixel);
                    encoded.push(pixel.to_array().map(|v| target.encode(v).unwrap()));
                }
            }
        }
        let transform = Transform::new_flags(
            &profile,
            PixelFormat::RGB_DBL,
            &Profile::new_xyz(),
            PixelFormat::XYZ_DBL,
            Intent::RelativeColorimetric,
            Flags::NO_OPTIMIZE,
        )
        .unwrap();
        let mut result = vec![[0.0; 3]; encoded.len()];
        transform.transform_pixels(&encoded, &mut result);
        for (linear, actual) in linear.into_iter().zip(result) {
            assert!(
                (DVec3::from_array(actual) - expected * linear)
                    .abs()
                    .max_element()
                    < 6e-5,
                "{target:?} {linear:?} {actual:?}"
            );
        }
    }
}

#[test]
fn sdr_targets_preserve_colored_grid_and_yellows_across_exposure() {
    let levels = [0., 0.01, 0.05, 0.1, 0.18, 0.2, 0.4, 0.5, 0.75, 1.];
    let mut grid = Vec::new();
    for r in levels {
        for g in levels {
            for b in levels {
                let input = DVec3::new(r, g, b);
                if input.max_element() - input.min_element() >= 0.009 {
                    grid.push(input);
                }
            }
        }
    }
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let renderer = CpuRenderer::new(target).unwrap();
        for (input, output) in grid.iter().zip(renderer.render(&grid).unwrap()) {
            assert!(
                output.max_element() - output.min_element() > 1e-6,
                "{target:?} {input:?} collapsed: {output:?}"
            );
        }
        for tint in -8..=8 {
            for blue in [0., 0.05, 0.25, 0.5] {
                let yellow = DVec3::new(
                    1. + f64::from(tint) * 0.01,
                    1. - f64::from(tint) * 0.01,
                    blue,
                );
                let inputs: Vec<_> = (0..=320)
                    .map(|i| yellow * 2f64.powf(-6. + f64::from(i) / 32.))
                    .collect();
                let outputs = renderer.render(&inputs).unwrap();
                for output in &outputs {
                    assert!(
                        output.x.min(output.y) > output.z + 1e-6,
                        "{target:?} {yellow:?} lost yellow: {output:?}"
                    );
                }
                for pair in outputs.windows(2) {
                    assert!(
                        (pair[1] - pair[0]).abs().max_element() < 0.025,
                        "{target:?} {yellow:?} exposure discontinuity: {pair:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn display_p3_preserves_displayable_colors_outside_srgb() {
    let target = OutputTarget::DisplayP3;
    let output = CpuRenderer::new(target)
        .unwrap()
        .render(&[DVec3::new(0., 1., 0.)])
        .unwrap()[0];
    let srgb = target.linear_to_srgb() * output.as_dvec3();
    assert!(
        srgb.min_element() < -0.01,
        "wide-gamut output was limited to sRGB: {srgb:?}"
    );
    let profile = Profile::new_icc(profiles::icc(target)).unwrap();
    assert!((target.rgb_to_xyz().col(0) - OutputTarget::Srgb.rgb_to_xyz().col(0)).length() > 0.05);
    assert!(profile.is_matrix_shaper());
}

#[test]
fn owned_ocio_processors_render_bulk_and_chunks_without_mutating_working_rgb() {
    assert_eq!(ocio_version(), "2.6.0");
    let mut inputs = vec![
        DVec3::ZERO,
        DVec3::splat(0.18),
        DVec3::ONE,
        DVec3::splat(1000.0),
        DVec3::new(-0.1, 2.0, 0.3),
    ];
    for r in -1..=5 {
        for g in -1..=5 {
            for b in -1..=5 {
                inputs.push(DVec3::new(
                    f64::from(r) * 0.2,
                    f64::from(g) * 0.2,
                    f64::from(b) * 0.2,
                ));
            }
        }
    }
    let original = inputs.clone();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let renderer = CpuRenderer::new(target).unwrap();
        assert!(renderer.render(&[]).unwrap().is_empty());
        let outputs = renderer.render(&inputs).unwrap();
        assert_eq!(inputs, original);
        assert!((outputs[1] - Vec3::splat(0.1)).abs().max_element() < 2e-6);
        assert!((outputs[2] - Vec3::splat(0.457566)).abs().max_element() < 2e-6);
        for (chunks, rendered) in inputs.chunks(11).zip(outputs.chunks(11)) {
            for (actual, expected) in renderer.render(chunks).unwrap().iter().zip(rendered) {
                assert!((*actual - *expected).abs().max_element() < 2e-5);
            }
        }
        assert!(matches!(
            renderer.render(&[DVec3::new(0.0, f64::NAN, 0.0)]),
            Err(Error::NonFinitePixel { channel: 1 })
        ));
        assert!(matches!(
            renderer.render(&[DVec3::splat(f64::MAX)]),
            Err(Error::NumericalOverflow)
        ));
        assert_eq!(renderer.render(&inputs).unwrap(), outputs);
        target.encode_u16(&outputs).unwrap();
    }
}
