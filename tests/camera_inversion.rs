use antitone::{
    color::{Balance, wb_matrix},
    processing::{
        Calibration, DEFAULT_GAMMA, InputSettings, Inversion, NegativeInput, PhotoSettings,
        Pipeline, Response, RollSettings,
    },
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

fn negative(cal: Calibration, response: Response, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response,
        gamma: DEFAULT_GAMMA,
        balance,
    }
}

fn input(negative: NegativeInput) -> InputSettings {
    InputSettings::Negative {
        dmin: Some(negative.calibration.dmin),
        dmax: Some(negative.calibration.dmax),
        input_floor: negative.calibration.input_floor,
        response: negative.response,
        gamma: negative.gamma,
        balance: negative.balance,
    }
}

fn forward(cal: Calibration, negative: NegativeInput, positive: DVec3) -> DVec3 {
    let (toe, shoulder) = negative.response.shape().unwrap();
    let gammas = negative.channel_gammas().unwrap();
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
    let interpretation = negative(cal, Response::film(), Balance::default());
    let roll = RollSettings {
        exposure: 1.25,
        wb: Balance {
            warmth: -0.35,
            tint: 0.15,
        },
    };
    let photo = PhotoSettings {
        exposure: -0.5,
        wb: Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        ..PhotoSettings::default()
    };
    let camera = DVec3::new(0.01, 0.1, 0.1);
    let inverted = Inversion::new(interpretation)
        .unwrap()
        .apply(camera)
        .unwrap();
    let actual = Pipeline::new(input(interpretation), roll, photo, matrix())
        .unwrap()
        .apply(camera)
        .unwrap();
    close(
        actual.rgb,
        wb_matrix(photo.wb).unwrap()
            * wb_matrix(roll.wb).unwrap()
            * matrix()
            * inverted.rgb
            * 0.75_f64.exp2(),
    );
    assert_eq!(actual.floored, inverted.floored);
    let mixed_calibration = Calibration {
        dmin: matrix() * cal.dmin,
        dmax: matrix() * cal.dmax,
        ..cal
    };
    let mixed = Inversion::new(negative(
        mixed_calibration,
        Response::film(),
        Balance::default(),
    ))
    .unwrap()
    .apply(matrix() * camera)
    .unwrap()
    .rgb;
    assert!((mixed - matrix() * inverted.rgb).abs().max_element() > 0.01);
}

#[test]
fn photo_neutral_is_a_replacement_residual_after_roll_wb_not_a_second_total_correction() {
    let cal = calibration();
    let total = Balance {
        warmth: 0.3,
        tint: -0.2,
    };
    let interpretation = negative(cal, Response::film(), Balance::default());
    let roll = RollSettings {
        wb: Balance {
            warmth: 0.12,
            tint: -0.03,
        },
        ..RollSettings::default()
    };
    let cast = wb_matrix(total).unwrap().inverse() * DVec3::splat(0.2);
    let scan = forward(cal, interpretation, matrix().inverse() * cast);
    let before_photo = Pipeline::new(
        input(interpretation),
        roll,
        PhotoSettings::default(),
        matrix(),
    )
    .unwrap()
    .apply(scan)
    .unwrap()
    .rgb;
    let residual = antitone::sampling::photo_neutral(&[before_photo]).unwrap();
    assert!((residual.warmth - (total.warmth - roll.wb.warmth)).abs() < 1e-11);
    assert!((residual.tint - (total.tint - roll.wb.tint)).abs() < 1e-11);
    let photo = PhotoSettings {
        wb: residual,
        ..PhotoSettings::default()
    };
    close(
        Pipeline::new(input(interpretation), roll, photo, matrix())
            .unwrap()
            .apply(scan)
            .unwrap()
            .rgb,
        DVec3::splat(0.2),
    );
}

#[test]
fn signed_working_values_do_not_set_camera_input_floor_flags() {
    let cal = calibration();
    let interpretation = negative(cal, Response::film(), Balance::default());
    let roll = RollSettings::default();
    let camera = forward(cal, interpretation, DVec3::new(0., 0.5, 0.001));
    let pipeline = Pipeline::new(
        input(interpretation),
        roll,
        PhotoSettings::default(),
        matrix(),
    )
    .unwrap();
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
        let truth = negative(
            cal,
            response,
            Balance {
                warmth: 0.2,
                tint: -0.1,
            },
        );
        let patch = [0.02, 0.05, 0.1, 0.25].map(|e| forward(cal, truth, target * e));
        let initial = negative(cal, response, Balance::default());
        let balance = roll_neutral(initial, &patch, matrix()).unwrap();
        assert!((balance.warmth - truth.balance.warmth).abs() < 1e-8);
        assert!((balance.tint - truth.balance.tint).abs() < 1e-8);
        let fitted = negative(cal, response, balance);
        let values: Vec<_> = patch
            .iter()
            .map(|p| Inversion::new(fitted).unwrap().apply(*p).unwrap().rgb)
            .collect();
        let neutral = matrix() * mean(&values).unwrap();
        assert!((neutral - DVec3::splat(0.105)).abs().max_element() < 1e-9);
        assert_eq!(initial.gamma, truth.gamma);
        assert_eq!(initial.response, truth.response);
    }
}

#[test]
fn roll_neutral_recovers_the_same_balance_across_dark_middle_and_light_patches() {
    let cal = Calibration {
        dmin: DVec3::new(0.14, 0.098, 0.094),
        dmax: DVec3::new(0.017, 0.0052, 0.0029),
        input_floor: 1. / 65535.,
    };
    let truth = negative(
        cal,
        Response::film(),
        Balance {
            warmth: -0.56,
            tint: -0.03,
        },
    );
    let initial = negative(cal, Response::film(), Balance::default());
    let target = matrix().inverse() * DVec3::ONE;
    let inverse = Inversion::new(truth).unwrap();
    let points: [_; 90] = std::array::from_fn(|i| {
        let y = 2_f64.powf(-7. + i as f64 / 89. * 6.8);
        let p = forward(cal, truth, target * y);
        assert!(p.min_element() >= cal.input_floor);
        assert!(
            (matrix() * inverse.apply(p).unwrap().rgb - DVec3::splat(y))
                .abs()
                .max_element()
                < 1e-11
        );
        p
    });
    for patch in [
        points.as_slice(),
        &points[..30],
        &points[30..60],
        &points[60..],
    ] {
        let fitted = roll_neutral(initial, patch, matrix()).unwrap();
        assert!((fitted.warmth - truth.balance.warmth).abs() < 1e-8);
        assert!((fitted.tint - truth.balance.tint).abs() < 1e-8);
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
        negative(cal, Response::Original, Balance::default()),
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
                RollSettings::default(),
                PhotoSettings::default(),
                invalid
            )
            .is_err()
        );
        assert!(roll_neutral(calibration(), &[DVec3::splat(0.05)], invalid).is_err());
    }
    assert!(
        roll_neutral(
            calibration(),
            &[DVec3::splat(0.05)],
            DMat3::from_diagonal(DVec3::new(1., -1., 1.))
        )
        .is_err()
    );
}
