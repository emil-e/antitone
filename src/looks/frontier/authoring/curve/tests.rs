use super::*;

#[test]
fn source_plateau_and_grey_ordinate_survive_monotone_interpolation() {
    for first in [32_u16, 36] {
        let values: Vec<_> = (0..1024)
            .map(|i| {
                if i < 64 {
                    first
                } else {
                    (first + 1 + (i - 64) as u16).min(960)
                }
            })
            .collect();
        let anchor = 410.25;
        let curve = Curve::from_runs(&values, anchor).unwrap();
        assert_eq!(curve.low, f64::from(first));
        for code in 0..=63 {
            assert_eq!(curve.sample(code as f64), f64::from(first));
        }
        assert_eq!(curve.sample(anchor), f64::from(values[410]) + 0.25);
        let mut previous = curve.sample(0.);
        for i in 1..=8184 {
            let value = curve.sample(i as f64 / 8.);
            assert!(value.is_finite() && value >= previous && value < 960.);
            previous = value;
        }
        let epsilon = 1e-5;
        assert!((curve.sample(63. + epsilon) - curve.sample(63.)) / epsilon < 1e-3);
    }
}

#[test]
fn invalid_or_constant_shapers_and_plateau_grey_anchors_reject() {
    assert!(Curve::from_runs(&[32; 1024], 410.).is_err());
    assert!(Curve::from_runs(&[960; 1024], 410.).is_err());
    let mut values: Vec<_> = (0..1024).map(|i| (i as u16).clamp(32, 960)).collect();
    assert!(Curve::from_runs(&values, 0.).is_err());
    values[500] = 100;
    assert!(Curve::from_runs(&values, 410.).is_err());
}
