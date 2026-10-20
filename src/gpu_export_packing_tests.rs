use super::*;

#[test]
fn streaming_readback_preserves_rgb16_codes_and_quantization_neighbors() {
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let mut pixels = Vec::new();
        for code in 0..=65535 {
            let linear = target.decode(f64::from(code) / 65535.).unwrap() as f32;
            pixels.push([linear, linear, linear, 0.]);
            if code < 65535 {
                let halfway = target.decode((f64::from(code) + 0.5) / 65535.).unwrap() as f32;
                pixels.push([
                    f32::from_bits(halfway.to_bits() - 1),
                    halfway,
                    f32::from_bits(halfway.to_bits() + 1),
                    0.,
                ]);
            }
        }
        pixels.extend([
            [-0., 0., 1., 0.],
            [
                f32::from_bits(1),
                0.0031308,
                f32::from_bits(0.0031308_f32.to_bits() + 1),
                0.,
            ],
        ]);
        // The scalar channel transfer/rounding is independent of the iterator layout adapter.
        let expected: Vec<[u16; 3]> = pixels
            .iter()
            .map(|p| {
                std::array::from_fn(|c| {
                    (target.encode(f64::from(p[c])).unwrap() * 65535.).round() as u16
                })
            })
            .collect();
        let actual = encode_readback(target, pixels).unwrap();
        assert_eq!(actual, expected);
        for code in 0..=65535 {
            assert_eq!(
                actual[code * 2],
                [code as u16; 3],
                "represented RGB16 code {code}"
            );
        }
    }
}

#[test]
fn readback_validation_retains_marker_priority_and_format_range_boundaries() {
    for channel in 0..4 {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut pixel = [0.25, 0.5, 0.75, 0.];
            pixel[channel] = invalid;
            assert!(matches!(
                encode_readback(OutputTarget::Srgb, vec![pixel]),
                Err(Error::Gpu(_))
            ));
            assert!(matches!(working_readback(vec![pixel]), Err(Error::Gpu(_))));
        }
    }
    let invalid_marker = [0.25, 0.5, 0.75, -1.];
    // Keep reconstruction corruption distinguishable from ordinary target-range rejection.
    assert!(matches!(
        encode_readback(OutputTarget::Srgb, vec![[2., 0., 0., 0.], invalid_marker]),
        Err(Error::Gpu(_))
    ));
    assert!(matches!(
        working_readback(vec![invalid_marker]),
        Err(Error::Gpu(_))
    ));
    for value in [-f32::from_bits(1), f32::from_bits(1_f32.to_bits() + 1)] {
        assert!(matches!(
            encode_readback(OutputTarget::DisplayP3, vec![[value, 0., 0., 0.]]),
            Err(Error::InvalidParameter(_))
        ));
    }
    let scene = vec![[-0.25, 2., -0., 0.], [f32::MAX, f32::MIN_POSITIVE, 1., 0.]];
    let actual = working_readback(scene.clone()).unwrap();
    for (pixel, expected) in actual.iter().zip(&scene) {
        for (channel, value) in pixel.to_array().into_iter().enumerate() {
            assert_eq!(value.to_bits(), f64::from(expected[channel]).to_bits());
        }
    }
}

#[test]
fn invalid_readback_preserves_existing_destination_for_tiff_and_both_exr_precisions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("destination");
    let composition =
        Composition::new([0, 0, 2, 1].into(), Default::default(), Default::default()).unwrap();
    for format in [
        Format::Tiff(OutputTarget::Srgb),
        Format::Exr(ExrPrecision::Half),
        Format::Exr(ExrPrecision::Full),
    ] {
        std::fs::write(&path, b"existing destination").unwrap();
        let mut published = false;
        let result = encode_with_checked(
            composition,
            format,
            crate::processing::Response::film().into(),
            (&path, Overwrite::Confirmed),
            |_, extent| {
                assert_eq!(extent, [2, 1]);
                Ok(vec![[0.25, 0.5, 0.75, 0.], [0.25, 0.5, 0.75, -1.]])
            },
            || {
                published = true;
                Ok(())
            },
        );
        assert!(matches!(result, Err(Error::Gpu(_))));
        assert!(!published);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "no failed-publication temporary remains"
        );
    }
}
