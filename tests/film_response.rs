use antitone::{
    color::Balance,
    processing::{Calibration, Inversion, PhotoSettings, Pipeline, Response, RollSettings},
    sampling::{mean, roll_neutral},
};
use glam::DVec3;

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.75, 0.48, 0.22),
        dmax: DVec3::new(0.003, 0.006, 0.008),
        input_floor: 1e-6,
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
fn forward(cal: Calibration, roll: RollSettings, exposure: DVec3) -> DVec3 {
    let (toe, k) = roll.response.shape().unwrap();
    let g = roll.channel_gammas().unwrap();
    DVec3::from_array(std::array::from_fn(|c| {
        let l = (cal.dmin[c] / cal.dmax[c]).ln();
        let a = toe * l / (g[c] - k * l / std::f64::consts::LN_10);
        let u = (exposure[c].signum() * exposure[c].abs().powf(toe) * a.exp_m1()).ln_1p();
        let density = g[c] * u / (toe + k * u / std::f64::consts::LN_10);
        cal.dmin[c] * (-density).exp()
    }))
}

#[test]
fn new_grades_opt_into_film_and_original_settings_remain_original() {
    assert_eq!(RollSettings::default().response, Response::Original);
    assert_eq!(
        RollSettings::new_grade().response,
        Response::Film {
            toe: 2.5,
            shoulder: 0.
        }
    );
    assert_eq!(Response::Original.processing_model(), 4);
    assert_eq!(Response::film().processing_model(), 4);
}
#[test]
fn film_one_zero_is_bit_identical_to_original_scalar_execution() {
    let cal = calibration();
    for gamma in [0.1, 0.65, 3.] {
        let original = RollSettings {
            gamma,
            ..RollSettings::default()
        };
        let film = RollSettings {
            response: Response::Film {
                toe: 1.,
                shoulder: 0.,
            },
            ..original
        };
        let a = Pipeline::new(
            cal,
            original,
            PhotoSettings::default(),
            glam::DMat3::IDENTITY,
        )
        .unwrap();
        let b = Pipeline::new(cal, film, PhotoSettings::default(), glam::DMat3::IDENTITY).unwrap();
        for scan in [
            cal.dmin,
            cal.dmax,
            cal.dmin * (1. + 1e-12),
            cal.dmin * (1. - 1e-12),
            cal.dmin * 0.3,
            DVec3::splat(-0.1),
        ] {
            assert_eq!(a.apply(scan).unwrap(), b.apply(scan).unwrap());
        }
    }
}
#[test]
fn film_anchors_forward_inverse_and_monotonic_floor_domain() {
    let cal = calibration();
    for toe in [1., 1.125, 2.5, 3.875, 4.] {
        for shoulder in [0., 0.005, 0.02] {
            let roll = RollSettings {
                response: Response::Film { toe, shoulder },
                ..RollSettings::default()
            };
            let inv = Inversion::new(cal, roll).unwrap();
            assert_eq!(inv.apply(cal.dmin).unwrap().rgb, DVec3::ZERO);
            assert_eq!(inv.apply(cal.dmax).unwrap().rgb, DVec3::ONE);
            for e in [0.002, 0.02, 0.2, 1., 2.] {
                let e = DVec3::splat(e);
                close(inv.apply(forward(cal, roll, e)).unwrap().rgb, e);
            }
            let mut previous = DVec3::ZERO;
            for i in 0..=256 {
                let scan =
                    cal.dmin * (DVec3::splat(cal.input_floor) / cal.dmin).powf(i as f64 / 256.);
                let v = inv.apply(scan).unwrap().rgb;
                assert!(v.is_finite() && v.cmpge(previous).all());
                let recovered = forward(cal, roll, v);
                for c in 0..3 {
                    let density = (cal.dmin[c] / scan[c]).log10();
                    let recovered_density = (cal.dmin[c] / recovered[c]).log10();
                    assert!((density - recovered_density).abs() <= 2e-12);
                }
                previous = v;
            }
            let floor = inv.apply(DVec3::splat(cal.input_floor)).unwrap();
            let below = inv.apply(DVec3::splat(-1.)).unwrap();
            assert_eq!(floor.rgb, below.rgb);
            assert_eq!(floor.floored, [false; 3]);
            assert_eq!(below.floored, [true; 3]);
        }
    }
}
#[test]
fn signed_continuation_has_no_pedestal_or_above_base_clamp() {
    let cal = calibration();
    for toe in [1., 2.5, 4.] {
        let roll = RollSettings {
            response: Response::Film {
                toe,
                shoulder: 0.02,
            },
            ..RollSettings::default()
        };
        let inv = Inversion::new(cal, roll).unwrap();
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
            close(forward(cal, roll, v.rgb), scan);
        }
    }
}
#[test]
fn invalid_shapes_nonfinite_inputs_and_floor_poles_are_rejected() {
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
            Inversion::new(
                cal,
                RollSettings {
                    response: Response::Film { toe, shoulder },
                    ..RollSettings::default()
                }
            )
            .is_err()
        );
    }
    assert!(
        Inversion::new(
            cal,
            RollSettings {
                gamma: 0.1,
                response: Response::Film {
                    toe: 2.5,
                    shoulder: 0.02
                },
                ..RollSettings::default()
            }
        )
        .is_err()
    );
    assert!(
        Inversion::new(cal, RollSettings::new_grade())
            .unwrap()
            .apply(DVec3::splat(f64::NAN))
            .is_err()
    );
}
#[test]
fn neutral_fit_uses_actual_response_and_keeps_unrelated_settings() {
    let cal = calibration();
    for response in [
        Response::Original,
        Response::film(),
        Response::Film {
            toe: 3.875,
            shoulder: 0.02,
        },
    ] {
        let truth = RollSettings {
            response,
            balance: Balance {
                warmth: 0.2,
                tint: -0.1,
            },
            exposure: 1.3,
            ..RollSettings::default()
        };
        let patch: Vec<_> = [0.02, 0.05, 0.1, 0.25]
            .map(|e| forward(cal, truth, DVec3::splat(e)))
            .into();
        let initial = RollSettings {
            balance: Balance::default(),
            ..truth
        };
        let balance = roll_neutral(cal, initial, &patch, glam::DMat3::IDENTITY).unwrap();
        assert!((balance.warmth - truth.balance.warmth).abs() < 1e-8);
        assert!((balance.tint - truth.balance.tint).abs() < 1e-8);
        let inv = Inversion::new(cal, RollSettings { balance, ..initial }).unwrap();
        let values: Vec<_> = patch.iter().map(|s| inv.apply(*s).unwrap().rgb).collect();
        let rgb = mean(&values).unwrap();
        assert!((rgb.max_element() - rgb.min_element()).abs() < 1e-9);
        assert_eq!(initial.response, response);
        assert_eq!(initial.gamma, truth.gamma);
        assert_eq!(initial.exposure, truth.exposure);
        assert!(roll_neutral(cal, initial, &[cal.dmin], glam::DMat3::IDENTITY).is_err());
        assert!(roll_neutral(cal, initial, &[DVec3::ZERO], glam::DMat3::IDENTITY).is_err());
    }
}
