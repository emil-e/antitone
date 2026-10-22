use antitone::lens::{EmbeddedLensProfile, LensCorrections};

#[test]
fn shading_uses_reciprocal_attenuation_at_shifted_knots_not_interpolated_gains() {
    let profile =
        EmbeddedLensProfile::from_knots(&[], &[], &[1., (-0.5f64).exp2(), (-1.5f64).exp2()])
            .unwrap();
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
