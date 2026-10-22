use antitone::lens::{EmbeddedLensProfile, LensCorrections};
use glam::{DVec2, DVec3};

#[test]
fn constant_profile_combines_independent_components_at_every_knot() {
    let distortion = vec![15. / 16.; 16];
    let ca = vec![DVec2::new(129. / 128., 63. / 64.); 16];
    let lens = EmbeddedLensProfile::from_knots(&distortion, &ca, &[]).unwrap();
    assert!(lens.has_distortion() && lens.has_ca());
    for distortion in [false, true] {
        for ca in [false, true] {
            let corrections = LensCorrections { distortion, ca };
            let scale = if distortion { 15. / 16. } else { 1. };
            let expected = if ca {
                DVec3::new(129. / 128., 1., 63. / 64.) * scale
            } else {
                DVec3::splat(scale)
            };
            for radius in [0., 0.01, 0.5, 1.] {
                assert_eq!(lens.inverse_scales(radius, corrections).unwrap(), expected);
            }
            let offset = DVec2::new(0.3, -0.4);
            let mapped = lens.inverse_map(offset, corrections).unwrap();
            for channel in 0..3 {
                assert_eq!(mapped[channel], offset * expected[channel]);
            }
            assert_eq!(
                lens.inverse_map(DVec2::ZERO, corrections).unwrap(),
                [DVec2::ZERO; 3]
            );
        }
    }
}

#[test]
fn interpolation_uses_shifted_radial_knots_and_combines_channels_at_each_knot() {
    let distortion: Vec<f64> = (0..16).map(|i| 1. + i as f64 / 64.).collect();
    let ca: Vec<DVec2> = (0..16)
        .map(|i| DVec2::new(1. + i as f64 / 16384., 1. - i as f64 / 8192.))
        .collect();
    let lens = EmbeddedLensProfile::from_knots(&distortion, &ca, &[]).unwrap();
    let expected = |i: f64| DVec3::new(1. + i / 16384., 1., 1. - i / 8192.) * (1. + i / 64.);
    for i in 0..15 {
        let radius = (i as f64 + 0.5) / 15.;
        let actual = lens.inverse_scales(radius, LensCorrections::ALL).unwrap();
        assert!((actual - expected(i as f64)).abs().max_element() < 1e-14);
    }
    for i in 0..15 {
        let radius = (i as f64 + 1.) / 15.;
        let expected = (expected(i as f64) + expected(i as f64 + 1.)) / 2.;
        let actual = lens.inverse_scales(radius, LensCorrections::ALL).unwrap();
        assert!((actual - expected).abs().max_element() < 1e-14);
    }
    assert_eq!(
        lens.inverse_scales(0.01, LensCorrections::ALL).unwrap(),
        DVec3::ONE
    );
}

#[test]
fn normalized_knots_reject_nonfinite_and_mismatched_component_counts() {
    for distortion in [vec![f64::NAN], vec![f64::INFINITY], vec![-1.]] {
        assert!(EmbeddedLensProfile::from_knots(&distortion, &[], &[]).is_err());
    }
    for ca in [vec![DVec2::new(f64::INFINITY, 1.)], vec![DVec2::NAN]] {
        assert!(EmbeddedLensProfile::from_knots(&[], &ca, &[]).is_err());
    }
    for attenuation in [vec![0.], vec![-1.], vec![f64::NAN]] {
        assert!(EmbeddedLensProfile::from_knots(&[], &[], &attenuation).is_err());
    }
    assert!(EmbeddedLensProfile::from_knots(&[1.], &[DVec2::ONE; 2], &[]).is_err());
}

#[test]
fn positive_scales_do_not_allow_a_radial_fold_between_knots() {
    assert!(EmbeddedLensProfile::from_knots(&[0.; 16], &[], &[]).is_err());
    let mut folding = vec![1.; 15];
    folding.push(0.5);
    assert!(EmbeddedLensProfile::from_knots(&folding, &[], &[]).is_err());
    let distortion = [1., 1. - 10922. / 16384.];
    let ca = [
        DVec2::new(1. + 32767. / 2097152., 1.),
        DVec2::new(1. - 1. / 64., 1.),
    ];
    assert!(EmbeddedLensProfile::from_knots(&distortion, &[], &[]).is_ok());
    assert!(EmbeddedLensProfile::from_knots(&[], &ca, &[]).is_ok());
    assert!(EmbeddedLensProfile::from_knots(&distortion, &ca, &[]).is_err());
}

#[test]
fn mapping_requires_a_finite_offset_inside_the_declared_calibrated_disc() {
    let lens = EmbeddedLensProfile::from_knots(&[1. + 128. / 16384.; 16], &[], &[]).unwrap();
    for radius in [-1., f64::NAN, f64::INFINITY, 1.0001] {
        assert!(lens.inverse_scales(radius, LensCorrections::ALL).is_err());
    }
    for point in [DVec2::new(1., 1.), DVec2::new(f64::NAN, 0.)] {
        assert!(lens.inverse_map(point, LensCorrections::ALL).is_err());
    }
}
