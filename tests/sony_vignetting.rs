use antitone::lens::{LensCorrections, SonyLensProfile};

#[test]
fn shading_uses_reciprocal_attenuation_at_shifted_knots_not_interpolated_gains() {
    let profile = SonyLensProfile::from_components(None, None, Some(&[3, 0, 8192, 16384])).unwrap();
    assert!(profile.has_vignetting());
    assert!(!profile.has_distortion() && !profile.has_ca());
    let sqrt2 = std::f64::consts::SQRT_2;
    for (radius, expected) in [
        (0., 1.),
        (0.25, 1.),
        (0.5, 2. / (1. + 1. / sqrt2)),
        (0.75, sqrt2),
        (1., 4. * sqrt2 / 3.),
    ] {
        assert!((profile.vignetting_gain(radius).unwrap() - expected).abs() < 1e-14);
    }
    assert!((profile.maximum_gain() - 2. * sqrt2).abs() < 1e-14);
    assert_eq!(
        profile.inverse_scales(1., LensCorrections::ALL).unwrap(),
        glam::DVec3::ONE
    );
    for radius in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
        assert!(profile.vignetting_gain(radius).is_err());
    }
}

#[test]
fn shading_is_independent_and_bounded_for_the_full_signed_coefficient_domain() {
    for coefficient in [i16::MIN, -8192, 0, 8192, i16::MAX] {
        let profile =
            SonyLensProfile::from_components(None, None, Some(&[2, coefficient, coefficient]))
                .unwrap();
        let expected = 2f64.powf(2f64.powf(coefficient as f64 / 8192. - 1.) - 0.5);
        for r in [0., 0.25, 0.5, 1.] {
            assert!((profile.vignetting_gain(r).unwrap() - expected).abs() < 1e-12);
        }
        assert!(expected > 0. && expected < antitone::processing::MAX_SOURCE_GAIN);
        assert!((profile.maximum_gain() - expected.max(1.)).abs() < 1e-12);
        assert_eq!(profile.has_vignetting(), coefficient != 0);
    }
    // The coefficient arrays are independently count-prefixed: shading is not a geometric lattice.
    let profile = SonyLensProfile::from_components(
        Some(&[2, -1024, -1024]),
        None,
        Some(&[3, 0, 8192, 16384]),
    )
    .unwrap();
    assert!(profile.has_distortion() && profile.has_vignetting());
    let mut padded = vec![11];
    padded.extend([0; 11]);
    padded.extend([i16::MAX; 5]);
    let zero = SonyLensProfile::from_components(None, None, Some(&padded)).unwrap();
    assert!(!zero.has_vignetting());
    assert_eq!(zero.maximum_gain(), 1.);
    for invalid in [vec![], vec![0], vec![1, 0], vec![3, 0, 0], vec![17; 18]] {
        assert!(SonyLensProfile::from_components(None, None, Some(&invalid)).is_err());
    }
}
