use antitone::{
    color::Balance,
    processing::{Calibration, DEFAULT_GAMMA, Inversion, NegativeInput, Response},
    sampling::{mean, roll_neutral},
};
use glam::DVec3;

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.75, 0.48, 0.22),
        dmax: DVec3::new(0.003, 0.006, 0.008),
    }
}

fn negative(cal: Calibration, response: Response, gamma: f64, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response,
        gamma,
        balance,
    }
}

fn close(a: DVec3, b: DVec3) {
    for c in 0..3 {
        assert!(
            (a[c] - b[c]).abs() <= 2e-12 + 2e-11 * b[c].abs(),
            "{a:?} != {b:?}"
        );
    }
}

fn forward(cal: Calibration, negative: NegativeInput, exposure: DVec3) -> DVec3 {
    let (toe, k) = negative.response.shape().unwrap();
    let g = negative.channel_gammas().unwrap();
    DVec3::from_array(std::array::from_fn(|c| {
        let l = (cal.dmin[c] / cal.dmax[c]).ln();
        let a = toe * l / (g[c] - k * l / std::f64::consts::LN_10);
        let u = (exposure[c].signum() * exposure[c].abs().powf(toe) * a.exp_m1()).ln_1p();
        let density = g[c] * u / (toe + k * u / std::f64::consts::LN_10);
        cal.dmin[c] * (-density).exp()
    }))
}

#[test]
fn response_defaults_are_explicit() {
    assert_eq!(
        Response::film(),
        Response {
            toe: 2.5,
            shoulder: 0.
        }
    );
    assert_eq!(DEFAULT_GAMMA, 0.60);
}

#[test]
fn film_anchors_and_ordinary_forward_inverse_are_preserved() {
    let cal = calibration();
    for toe in [1., 1.125, 2.5, 3.875, 4.] {
        for shoulder in [0., 0.005, 0.02] {
            let neg = negative(
                cal,
                Response { toe, shoulder },
                DEFAULT_GAMMA,
                Balance::default(),
            );
            let inv = Inversion::new(neg).unwrap();
            assert_eq!(inv.apply(cal.dmin).unwrap().rgb, DVec3::ZERO);
            assert_eq!(inv.apply(cal.dmax).unwrap().rgb, DVec3::ONE);
            for e in [0.002, 0.02, 0.2, 1., 2.] {
                let e = DVec3::splat(e);
                close(inv.apply(forward(cal, neg, e)).unwrap().rgb, e);
            }
            let mut previous = DVec3::ZERO;
            for i in 0..=256 {
                let scan = cal.dmin * (cal.dmax / (16. * cal.dmin)).powf(i as f64 / 256.);
                let v = inv.apply(scan).unwrap().rgb;
                assert!(v.is_finite() && v.cmpge(previous).all());
                let recovered = forward(cal, neg, v);
                for c in 0..3 {
                    let density = (cal.dmin[c] / scan[c]).log10();
                    let recovered_density = (cal.dmin[c] / recovered[c]).log10();
                    assert!((density - recovered_density).abs() <= 2e-12);
                }
                previous = v;
            }
            let zero = inv.apply(DVec3::ZERO).unwrap();
            let below = inv.apply(DVec3::splat(-1.)).unwrap();
            assert_eq!(zero.rgb, below.rgb);
            assert!(zero.rgb.is_finite() && zero.rgb.cmpgt(previous).all());
            assert_eq!(below.floored, [true; 3]);
        }
    }
}

#[test]
fn signed_continuation_has_no_pedestal_or_above_base_clamp() {
    let cal = calibration();
    for toe in [1., 2.5, 4.] {
        let neg = negative(
            cal,
            Response {
                toe,
                shoulder: 0.02,
            },
            DEFAULT_GAMMA,
            Balance::default(),
        );
        let inv = Inversion::new(neg).unwrap();
        for epsilon in [-1e-7, -1e-12, 0., 1e-12, 1e-7] {
            let scan = cal.dmin * (1. + epsilon);
            let v = inv.apply(scan).unwrap();
            assert_eq!(v.floored, [false; 3]);
            assert!(v.rgb.is_finite());
            for c in 0..3 {
                assert_eq!(
                    v.rgb[c].signum(),
                    if epsilon == 0. {
                        0_f64.signum()
                    } else {
                        -epsilon.signum()
                    }
                );
            }
            close(forward(cal, neg, v.rgb), scan);
        }
    }
}

#[test]
fn invalid_shapes_and_nonfinite_samples_have_no_photographic_interpretation() {
    let cal = calibration();
    for (toe, shoulder) in [
        (0.99, 0.),
        (4.01, 0.),
        (f64::NAN, 0.),
        (2.5, -0.001),
        (2.5, 0.0201),
        (2.5, f64::INFINITY),
    ] {
        assert!(
            Inversion::new(negative(
                cal,
                Response { toe, shoulder },
                DEFAULT_GAMMA,
                Balance::default()
            ))
            .is_err()
        );
    }
    assert!(
        Inversion::new(negative(
            cal,
            Response::film(),
            DEFAULT_GAMMA,
            Balance::default()
        ))
        .unwrap()
        .apply(DVec3::splat(f64::NAN))
        .is_err()
    );
}

#[test]
fn inversion_is_independent_of_camera_units_and_source_quantization() {
    let calibration = Calibration {
        dmin: DVec3::new(0.8, 0.4, 0.2),
        dmax: DVec3::new(0.002, 0.003, 0.004),
    };
    let response = Response {
        toe: 2.5,
        shoulder: 0.02,
    };
    let reference =
        Inversion::new(negative(calibration, response, 0.6, Balance::default())).unwrap();
    for scale in [1e-8, 0.25, 1., 65535., 1e8] {
        let scaled = Calibration {
            dmin: calibration.dmin * scale,
            dmax: calibration.dmax * scale,
        };
        let inversion =
            Inversion::new(negative(scaled, response, 0.6, Balance::default())).unwrap();
        for sample in [
            calibration.dmin * 1.1,
            calibration.dmin,
            calibration.dmax,
            calibration.dmax * 0.1,
            DVec3::splat(1e-12),
            DVec3::ZERO,
        ] {
            close(
                inversion.apply(sample * scale).unwrap().rgb,
                reference.apply(sample).unwrap().rgb,
            );
        }
    }
}

#[test]
fn zero_light_has_a_finite_tangent_continuation_not_a_source_code_floor() {
    // For toe=gamma=1 and no shoulder, the anchored inverse is (1/s - 1)/99.
    let calibration = Calibration {
        dmin: DVec3::ONE,
        dmax: DVec3::splat(0.01),
    };
    let inversion = Inversion::new(negative(
        calibration,
        Response {
            toe: 1.,
            shoulder: 0.,
        },
        1.,
        Balance::default(),
    ))
    .unwrap();
    let white = 65536.;
    let join = 1. / (1. + 99. * white);
    let slope = white + 1. / 99.;
    for fraction in [1., 0.75, 0.5, 0.25, 0.] {
        close(
            inversion.apply(DVec3::splat(join * fraction)).unwrap().rgb,
            DVec3::splat(white + slope * (1. - fraction)),
        );
    }
    let epsilon = join * 1e-5;
    let at = inversion.apply(DVec3::splat(join)).unwrap().rgb.x;
    let left = inversion.apply(DVec3::splat(join - epsilon)).unwrap().rgb.x;
    let right = inversion.apply(DVec3::splat(join + epsilon)).unwrap().rgb.x;
    assert!(((left - at) / (at - right) - 1.).abs() < 2e-5);
}

#[test]
fn shoulder_continues_with_matching_slope_before_the_old_pole() {
    let gamma = 0.1;
    let shoulder = 0.02;
    let h = shoulder / std::f64::consts::LN_10;
    let density = gamma / (2. * h);
    let join = (-density).exp();
    let calibration = Calibration {
        dmin: DVec3::ONE,
        dmax: DVec3::splat(join),
    };
    let inversion = Inversion::new(negative(
        calibration,
        Response { toe: 2.5, shoulder },
        gamma,
        Balance::default(),
    ))
    .unwrap();
    close(inversion.apply(calibration.dmax).unwrap().rgb, DVec3::ONE);
    let epsilon = join * 1e-6;
    let left = inversion.apply(DVec3::splat(join - epsilon)).unwrap().rgb.x;
    let right = inversion.apply(DVec3::splat(join + epsilon)).unwrap().rgb.x;
    assert!(((left - 1.) / (1. - right) - 1.).abs() < 1e-4);
    let mut previous = 0.;
    for i in 0..=1000 {
        let sample = (-(i as f64) * 0.1).exp();
        let value = inversion.apply(DVec3::splat(sample)).unwrap().rgb.x;
        assert!(value.is_finite() && value >= previous);
        previous = value;
    }
    assert!(inversion.apply(DVec3::ZERO).unwrap().rgb.x >= previous);
}

#[test]
fn neutral_fit_uses_actual_response_and_keeps_unrelated_settings() {
    let cal = calibration();
    for response in [
        Response::film(),
        Response {
            toe: 3.875,
            shoulder: 0.02,
        },
    ] {
        let truth = negative(
            cal,
            response,
            DEFAULT_GAMMA,
            Balance {
                warmth: 0.2,
                tint: -0.1,
            },
        );
        let patch: Vec<_> = [0.02, 0.05, 0.1, 0.25]
            .map(|e| forward(cal, truth, DVec3::splat(e)))
            .into();
        let initial = negative(cal, response, DEFAULT_GAMMA, Balance::default());
        let balance = roll_neutral(initial, &patch, glam::DMat3::IDENTITY).unwrap();
        assert!((balance.warmth - truth.balance.warmth).abs() < 1e-8);
        assert!((balance.tint - truth.balance.tint).abs() < 1e-8);
        let fitted = negative(cal, response, DEFAULT_GAMMA, balance);
        let values: Vec<_> = patch
            .iter()
            .map(|s| Inversion::new(fitted).unwrap().apply(*s).unwrap().rgb)
            .collect();
        let rgb = mean(&values).unwrap();
        assert!((rgb.max_element() - rgb.min_element()).abs() < 1e-9);
        assert_eq!(initial.response, response);
        assert_eq!(initial.gamma, DEFAULT_GAMMA);
        assert!(roll_neutral(initial, &[cal.dmin], glam::DMat3::IDENTITY).is_err());
        assert!(roll_neutral(initial, &[DVec3::ZERO], glam::DMat3::IDENTITY).is_err());
    }
}
