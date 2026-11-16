use super::*;

#[test]
fn colour_matrix_preserves_the_fitted_neutral_trajectory_before_channel_gains() {
    let entry = Entry::calibrated(Variant::A);
    let mut identity = entry.clone();
    identity.matrix = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for row in entry.matrix {
        assert!((row.iter().sum::<f64>() - 1.).abs() < 2. * f64::EPSILON);
    }
    for ev in -48..=48 {
        let rgb = DVec3::splat(0.18 * 2_f64.powf(ev as f64 / 4.));
        assert!(
            (entry.coordinates(rgb).unwrap() - identity.coordinates(rgb).unwrap())
                .abs()
                .max_element()
                < 2e-12
        );
    }
    assert!(
        (entry.coordinates(DVec3::new(0.2, 0.1, 0.05)).unwrap()
            - identity.coordinates(DVec3::new(0.2, 0.1, 0.05)).unwrap())
        .abs()
        .max_element()
            > 1.
    );
}

#[test]
fn exposure_and_channel_gain_changes_preserve_the_black_anchor() {
    for variant in Variant::all() {
        let mut entry = Entry::calibrated(variant);
        for gain in [0.1, 1., 10.] {
            entry.knee *= gain;
            entry.channel_gains = [gain, 1., 1. / gain];
            assert_eq!(
                entry.coordinates(DVec3::ZERO).unwrap(),
                DVec3::splat(variant.code_offset())
            );
            assert_eq!(
                entry.coordinates(DVec3::splat(-0.01)).unwrap(),
                DVec3::splat(variant.code_offset())
            );
        }
    }
}

#[test]
fn b_origin_translates_formed_codes_before_upper_continuation() {
    let a = Entry::calibrated(Variant::A);
    let b = Entry::calibrated(Variant::B);
    assert_eq!(a.matrix, b.matrix);
    assert_eq!(a.codes_per_stop, b.codes_per_stop);
    assert_eq!(a.knee, b.knee);
    assert_eq!(a.toe, b.toe);
    assert_eq!(a.channel_gains, b.channel_gains);
    for signal in [0., 1e-9, 0.01, 0.18, 1., 1e6] {
        let qa = a.raw_code(signal);
        let qb = b.raw_code(signal);
        let bound = 8. * f64::EPSILON * qb.abs().max(62.);
        assert!((qb - qa - 62.).abs() <= bound);
        let expected = if qb >= 1015. {
            1023. - 64. / (qb - 1007.)
        } else {
            qb
        };
        assert_eq!(b.code(signal), expected);
    }
    assert!(b.code(1e6) - a.code(1e6) < 62.);
}
