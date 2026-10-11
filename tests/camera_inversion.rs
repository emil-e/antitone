use antitone::{
    color::{Balance, wb_matrix},
    processing::{Calibration, Inversion, PhotoSettings, Pipeline, Response, RollSettings},
    sampling::{mean, roll_neutral},
};
use glam::{DMat3, DVec3};

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.75, 0.48, 0.22),
        dmax: DVec3::new(0.003, 0.006, 0.008),
        input_floor: 1. / 65535.,
    }
}
fn matrix() -> DMat3 {
    DMat3::from_cols(
        DVec3::new(1.1, 0.03, -0.07),
        DVec3::new(0.2, 0.8, -0.1),
        DVec3::new(-0.03, 0.08, 1.3),
    )
}
fn forward(cal: Calibration, roll: RollSettings, positive: DVec3) -> DVec3 {
    let (toe, shoulder) = roll.response.shape().unwrap();
    let gammas = roll.channel_gammas().unwrap();
    DVec3::from_array(std::array::from_fn(|c| {
        let l = (cal.dmin[c] / cal.dmax[c]).ln();
        let anchor = toe * l / (gammas[c] - shoulder * l / std::f64::consts::LN_10);
        let u = (positive[c].powf(toe) * anchor.exp_m1()).ln_1p();
        let density = gammas[c] * u / (toe + shoulder * u / std::f64::consts::LN_10);
        cal.dmin[c] * (-density).exp()
    }))
}
fn close(a: DVec3, b: DVec3) {
    assert!(
        (a - b).abs().max_element() <= 2e-12 + 2e-11 * b.abs().max_element(),
        "{a:?} != {b:?}"
    );
}

#[test]
fn camera_matrix_follows_inversion_and_precedes_photographic_wb() {
    let cal = calibration();
    let roll = RollSettings {
        exposure: 1.25,
        ..RollSettings::new_grade()
    };
    let photo = PhotoSettings {
        exposure: -0.5,
        wb: Balance {
            warmth: 0.2,
            tint: -0.1,
        },
    };
    let camera = DVec3::new(0.01, 0.1, 0.1);
    let inverted = Inversion::new(cal, roll).unwrap().apply(camera).unwrap();
    let actual = Pipeline::new(cal, roll, photo, matrix())
        .unwrap()
        .apply(camera)
        .unwrap();
    close(
        actual.rgb,
        wb_matrix(photo.wb).unwrap() * matrix() * inverted.rgb * 0.75_f64.exp2(),
    );
    assert_eq!(actual.floored, inverted.floored);
    let mixed_calibration = Calibration {
        dmin: matrix() * cal.dmin,
        dmax: matrix() * cal.dmax,
        ..cal
    };
    let mixed = Inversion::new(mixed_calibration, roll)
        .unwrap()
        .apply(matrix() * camera)
        .unwrap()
        .rgb;
    assert!((mixed - matrix() * inverted.rgb).abs().max_element() > 0.01);
}

#[test]
fn signed_working_values_do_not_set_camera_input_floor_flags() {
    let cal = calibration();
    let roll = RollSettings::new_grade();
    let camera = forward(cal, roll, DVec3::new(0., 0.5, 0.001));
    let pipeline = Pipeline::new(cal, roll, PhotoSettings::default(), matrix()).unwrap();
    let positive = pipeline.apply(camera).unwrap();
    assert!(positive.rgb.z < 0.);
    assert_eq!(positive.floored, [false; 3]);
    assert_eq!(
        pipeline.apply(DVec3::new(0., 0.05, 0.08)).unwrap().floored,
        [true, false, false]
    );
}

#[test]
fn roll_neutral_fits_actual_response_to_unequal_camera_white_not_equal_channels() {
    let cal = calibration();
    let target = matrix().inverse() * DVec3::ONE;
    assert!(target.min_element() > 0. && target.max_element() - target.min_element() > 0.1);
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
        let patch = [0.02, 0.05, 0.1, 0.25].map(|e| forward(cal, truth, target * e));
        let initial = RollSettings {
            balance: Balance::default(),
            ..truth
        };
        let balance = roll_neutral(cal, initial, &patch, matrix()).unwrap();
        assert!((balance.warmth - truth.balance.warmth).abs() < 1e-8);
        assert!((balance.tint - truth.balance.tint).abs() < 1e-8);
        let inverse = Inversion::new(cal, RollSettings { balance, ..initial }).unwrap();
        let values: Vec<_> = patch
            .iter()
            .map(|p| inverse.apply(*p).unwrap().rgb)
            .collect();
        let neutral = matrix() * mean(&values).unwrap();
        assert!((neutral - DVec3::splat(0.105)).abs().max_element() < 1e-9);
        assert_eq!(initial.exposure, truth.exposure);
        assert_eq!(initial.gamma, truth.gamma);
        assert_eq!(initial.response, truth.response);
    }
}

#[cfg(feature = "gpu")]
#[test]
fn gpu_range_gate_covers_above_base_continuation_after_camera_mapping() {
    let cal = Calibration {
        dmin: DVec3::splat(0.001),
        dmax: DVec3::splat(0.001 * (1. - 1.1e-6)),
        input_floor: 0.001 * (1. - 1.1e-6),
    };
    let result = antitone::gpu::ProcessingParameters::positive(
        cal,
        RollSettings::default(),
        PhotoSettings::default(),
        DMat3::from_diagonal(DVec3::splat(3e32)),
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("response domain")
    );
}

#[test]
fn invalid_matrix_or_nonpositive_camera_neutral_is_rejected() {
    for invalid in [DMat3::ZERO, DMat3::from_diagonal(DVec3::splat(f64::NAN))] {
        assert!(
            Pipeline::new(
                calibration(),
                RollSettings::new_grade(),
                PhotoSettings::default(),
                invalid
            )
            .is_err()
        );
        assert!(
            roll_neutral(
                calibration(),
                RollSettings::new_grade(),
                &[DVec3::splat(0.05)],
                invalid
            )
            .is_err()
        );
    }
    assert!(
        roll_neutral(
            calibration(),
            RollSettings::new_grade(),
            &[DVec3::splat(0.05)],
            DMat3::from_diagonal(DVec3::new(1., -1., 1.))
        )
        .is_err()
    );
}
