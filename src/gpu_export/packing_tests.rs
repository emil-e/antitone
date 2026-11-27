use super::*;

#[test]
fn linear_readback_rejects_invalid_contributors_and_preserves_signed_headroom() {
    for channel in 0..4 {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut pixel = [0.25, 0.5, 0.75, 0.];
            pixel[channel] = invalid;
            assert!(matches!(linear_readback(&pixel), Err(Error::Gpu(_))));
        }
    }
    assert!(matches!(
        linear_readback(&[0.25, 0.5, 0.75, -1.]),
        Err(Error::Gpu(_))
    ));

    let scene = vec![[-0.25, 2., -0., 0.], [f32::MAX, f32::MIN_POSITIVE, 1., 0.]];
    let actual = scene
        .iter()
        .map(linear_readback)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (pixel, expected) in actual.iter().zip(&scene) {
        for (channel, value) in pixel.to_array().into_iter().enumerate() {
            assert_eq!(value.to_bits(), f64::from(expected[channel]).to_bits());
        }
    }
}

#[test]
fn invalid_linear_readback_preserves_existing_destination_for_jpeg_and_both_exr_precisions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("destination");
    let composition =
        Composition::new([0, 0, 2, 1].into(), Default::default(), Default::default()).unwrap();
    let jpeg = Plan::new(
        RenderTarget::Display(OutputTarget::Srgb),
        export::FileFormat::Jpeg(export::JpegQuality::default()),
    )
    .unwrap();
    for plan in [
        jpeg,
        Format::Exr(ExrPrecision::Half).plan(),
        Format::Exr(ExrPrecision::Full).plan(),
    ] {
        std::fs::write(&path, b"existing destination").unwrap();
        let mut published = false;
        let result = encode_with_checked(
            composition,
            plan,
            (&path, Overwrite::Confirmed),
            |_, extent| {
                assert_eq!(extent, [2, 1]);
                Ok(OutputBlock::Linear(vec![
                    [0.25, 0.5, 0.75, 0.],
                    [0.25, 0.5, 0.75, -1.],
                ]))
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
