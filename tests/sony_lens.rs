use antitone::lens::{LensCorrections, SonyLensProfile};
use glam::{DVec2, DVec3};

fn parameters(count: usize, value: i16) -> Vec<i16> {
    std::iter::once(count as i16)
        .chain(std::iter::repeat_n(value, count))
        .collect()
}

#[test]
fn constant_profile_uses_known_binary_units_and_independent_components() {
    let distortion = parameters(16, -1024);
    let ca: Vec<_> = std::iter::once(32)
        .chain(std::iter::repeat_n(16384, 16))
        .chain(std::iter::repeat_n(-32768, 16))
        .collect();
    let lens = SonyLensProfile::from_parameters(Some(&distortion), Some(&ca)).unwrap();
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
    let distortion: Vec<i16> = std::iter::once(16)
        .chain((0..16).map(|i| i * 256))
        .collect();
    let ca: Vec<i16> = std::iter::once(32)
        .chain((0..16).map(|i| i * 128))
        .chain((0..16).map(|i| -i * 256))
        .collect();
    let lens = SonyLensProfile::from_parameters(Some(&distortion), Some(&ca)).unwrap();
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
fn absent_zero_and_padded_components_remain_distinct_from_malformed_parameters() {
    let absent = SonyLensProfile::from_parameters(None, None).unwrap();
    assert!(!absent.has_distortion() && !absent.has_ca());
    assert_eq!(absent.knot_count(), 0);
    assert_eq!(
        absent.inverse_scales(1., LensCorrections::ALL).unwrap(),
        DVec3::ONE
    );
    let zero = SonyLensProfile::from_parameters(Some(&parameters(16, 0)), None).unwrap();
    assert!(!zero.has_distortion() && !zero.has_ca());
    assert_eq!(zero.knot_count(), 16);
    let mut padded = parameters(11, 256);
    padded.extend([i16::MAX; 5]);
    let lens = SonyLensProfile::from_parameters(Some(&padded), None).unwrap();
    assert_eq!(lens.knot_count(), 11);
    assert_eq!(
        lens.inverse_scales(1., LensCorrections::ALL).unwrap(),
        DVec3::splat(65. / 64.)
    );
    let ca = parameters(22, 128);
    let ca_only = SonyLensProfile::from_parameters(None, Some(&ca)).unwrap();
    assert!(ca_only.has_ca() && !ca_only.has_distortion());
    assert_eq!(ca_only.knot_count(), 11);
}

#[test]
fn invalid_counts_lengths_and_component_lattices_are_rejected() {
    for data in [vec![], vec![0], vec![-1], vec![1, 0], parameters(17, 0)] {
        assert!(SonyLensProfile::from_parameters(Some(&data), None).is_err());
    }
    for data in [parameters(3, 0), parameters(33, 0), parameters(31, 0)] {
        assert!(SonyLensProfile::from_parameters(None, Some(&data)).is_err());
    }
    let mut truncated = parameters(16, 0);
    truncated.pop();
    assert!(SonyLensProfile::from_parameters(Some(&truncated), None).is_err());
    assert!(
        SonyLensProfile::from_parameters(Some(&parameters(16, 0)), Some(&parameters(22, 0)))
            .is_err()
    );
}

#[test]
fn positive_scales_do_not_allow_a_radial_fold_between_knots() {
    assert!(SonyLensProfile::from_parameters(Some(&parameters(16, -16384)), None).is_err());
    let mut folding = parameters(16, 0);
    folding[15] = -8192;
    assert!(SonyLensProfile::from_parameters(Some(&folding), None).is_err());
    let distortion = [2, 0, -10922];
    let ca = [4, 32767, -32768, 0, 0];
    assert!(SonyLensProfile::from_parameters(Some(&distortion), None).is_ok());
    assert!(SonyLensProfile::from_parameters(None, Some(&ca)).is_ok());
    assert!(SonyLensProfile::from_parameters(Some(&distortion), Some(&ca)).is_err());
}

#[test]
fn mapping_requires_a_finite_offset_inside_the_declared_calibrated_disc() {
    let lens = SonyLensProfile::from_parameters(Some(&parameters(16, 128)), None).unwrap();
    for radius in [-1., f64::NAN, f64::INFINITY, 1.0001] {
        assert!(lens.inverse_scales(radius, LensCorrections::ALL).is_err());
    }
    for point in [DVec2::new(1., 1.), DVec2::new(f64::NAN, 0.)] {
        assert!(lens.inverse_map(point, LensCorrections::ALL).is_err());
    }
}
