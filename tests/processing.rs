use antitone::Error;
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::color::{
    AP0_TO_XYZ, AP1_TO_XYZ, BRADFORD, Balance, ap0_to_ap1, ap1_to_ap0, ap1_to_cones, neutral_wb,
    wb_matrix,
};
use antitone::processing::{
    Calibration, DEFAULT_GAMMA, InputPipeline, InputSettings, Inversion, NegativeInput, Response,
    RollSettings, exposure_multiplier,
};
use antitone::sampling::{mean, roll_neutral, weighted_mean, weighted_mean_iter, white_balance};
use glam::{DMat3, DVec3};

/// A negative interpretation over measured anchors. The response is the identity (toe 1, no
/// shoulder) because these oracles express the plain anchored gamma formula.
fn neg(cal: Calibration, gamma: f64, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
        gamma,
        balance,
    }
}

/// The same interpretation as a source input stage (owns the anchors).
fn inp(cal: Calibration, gamma: f64, balance: Balance) -> InputSettings {
    InputSettings::Negative {
        dmin: Some(cal.dmin),
        dmax: Some(cal.dmax),
        input_floor: cal.input_floor,
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
        gamma,
        balance,
    }
}

#[test]
fn ungraded_positive_excludes_photographic_roll_controls_but_preserves_inversion_and_signed_hdr() {
    let cal = calibration();
    let matrix = DMat3::from_diagonal(DVec3::new(0.8, 1.1, 1.3));
    let gamma = 0.75;
    let balance = Balance {
        warmth: 0.06,
        tint: -0.03,
    };
    // The grade (roll exposure + WB) must not reach the ungraded positive.
    let graded_roll = RollSettings {
        exposure: 3.,
        wb: Balance {
            warmth: -0.2,
            tint: 0.1,
        },
    };
    let neutral = InputPipeline::ungraded(inp(cal, gamma, balance), matrix).unwrap();
    let reference = Inversion::new(neg(cal, gamma, balance)).unwrap();
    for input in [cal.dmin * 1.1, cal.dmin, cal.dmax, cal.dmax * 0.5] {
        let actual = neutral.apply(input).unwrap();
        let inverted = reference.apply(input).unwrap();
        assert_eq!(actual.floored, inverted.floored);
        assert_eq!(actual.rgb, matrix * inverted.rgb);
        assert_eq!(
            actual,
            InputPipeline::ungraded(inp(cal, gamma, balance), matrix)
                .unwrap()
                .apply(input)
                .unwrap()
        );
    }
    // The grade is wired separately and does reach the graded pipeline.
    let graded = InputPipeline::new(inp(cal, gamma, balance), graded_roll, matrix).unwrap();
    let mid = cal.dmax * 0.5;
    assert_ne!(
        graded.apply(mid).unwrap().rgb,
        neutral.apply(mid).unwrap().rgb
    );
    assert!(neutral.apply(cal.dmin * 1.1).unwrap().rgb.min_element() < 0.);
    assert!(neutral.apply(cal.dmax * 0.5).unwrap().rgb.max_element() > 1.);
}

#[test]
fn digital_input_is_already_positive_and_reuses_the_shared_grade() {
    let matrix = DMat3::from_diagonal(DVec3::new(0.8, 1.1, 1.3));
    // Identity grade + neutral roll WB + zero exposure: apply is camera→AP1 only,
    // with no polarity flip (the sample is already a scene-referred positive).
    let pipeline =
        InputPipeline::new(InputSettings::Digital, RollSettings::default(), matrix).unwrap();
    for input in [
        DVec3::splat(0.18),
        DVec3::new(-0.02, 0.5, 1.7),
        DVec3::splat(0.0),
    ] {
        let actual = pipeline.apply(input).unwrap();
        assert_eq!(actual.floored, [false; 3]);
        assert_eq!(actual.rgb, matrix * input);
    }
    // Ungraded digital positive is camera→AP1, before roll/photo controls.
    let neutral = InputPipeline::ungraded(InputSettings::Digital, matrix).unwrap();
    assert_eq!(
        neutral.apply(DVec3::splat(0.18)).unwrap().rgb,
        matrix * DVec3::splat(0.18)
    );
    // The shared grade reaches the digital positive: roll exposure scales the working value.
    let stops = 1.5;
    let graded = InputPipeline::new(
        InputSettings::Digital,
        RollSettings {
            exposure: stops,
            ..RollSettings::default()
        },
        matrix,
    )
    .unwrap();
    let input = DVec3::splat(0.18);
    close_rgb(
        graded.apply(input).unwrap().rgb,
        matrix * input * exposure_multiplier(stops).unwrap(),
    );
    // Non-finite samples reject rather than becoming an edge colour.
    assert!(pipeline.apply(DVec3::splat(f64::NAN)).is_err());
    assert!(pipeline.apply(DVec3::new(f64::INFINITY, 0., 0.)).is_err());
}

#[test]
fn weighted_neutral_picks_match_repeated_full_resolution_pixels_not_bounding_box_means() {
    use antitone::sampling::{roll_neutral_weighted, white_balance_weighted};
    let cal = calibration();
    let correct = Balance {
        warmth: 0.21,
        tint: -0.14,
    };
    let matrix = DMat3::from_diagonal(DVec3::new(0.8, 1.1, 1.3));
    let target = matrix.inverse() * DVec3::ONE;
    let patch = [0.025, 0.09, 0.4]
        .map(|v| scan_for_positive(cal, neg(cal, DEFAULT_GAMMA, correct), target * v));
    let weighted = [(patch[0], 0.5), (patch[1], 1.), (patch[2], 0.5)];
    let repeated = [patch[0], patch[1], patch[1], patch[2]];
    assert!((weighted_mean(&weighted).unwrap() - mean(&repeated).unwrap()).length() < 1e-15);

    let weighted_balance = white_balance_weighted(&weighted).unwrap();
    let repeated_balance = white_balance(&repeated).unwrap();
    assert!((weighted_balance.warmth - repeated_balance.warmth).abs() < 1e-14);
    assert!((weighted_balance.tint - repeated_balance.tint).abs() < 1e-14);

    let interpretation = neg(cal, DEFAULT_GAMMA, Balance::default());
    let weighted_roll_balance = roll_neutral_weighted(interpretation, &weighted, matrix).unwrap();
    let repeated_roll_balance = roll_neutral(interpretation, &repeated, matrix).unwrap();
    assert!((weighted_roll_balance.warmth - repeated_roll_balance.warmth).abs() < 1e-9);
    assert!((weighted_roll_balance.tint - repeated_roll_balance.tint).abs() < 1e-9);
    assert!((weighted_roll_balance.warmth - correct.warmth).abs() < 1e-8);
    assert!((weighted_roll_balance.tint - correct.tint).abs() < 1e-8);

    assert!(weighted_mean(&[(patch[0], 0.)]).is_err());
    assert!(weighted_mean(&[(patch[0], f64::NAN)]).is_err());
}

const ABS: f64 = 2e-12;
const REL: f64 = 2e-11;

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= ABS + REL * expected.abs(),
        "{actual} != {expected}"
    );
}

fn close_rgb(actual: DVec3, expected: DVec3) {
    for c in 0..3 {
        close(actual[c], expected[c]);
    }
}

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.75, 0.48, 0.22),
        dmax: DVec3::new(0.003, 0.006, 0.008),
        input_floor: 1e-6,
    }
}

fn scan_for_positive(
    calibration: Calibration,
    interpretation: NegativeInput,
    value: DVec3,
) -> DVec3 {
    let gamma = interpretation.channel_gammas().unwrap();
    DVec3::from_array(std::array::from_fn(|c| {
        let denominator = (calibration.dmin[c] / calibration.dmax[c]).powf(1.0 / gamma[c]) - 1.0;
        calibration.dmin[c] / (1.0 + value[c] * denominator).powf(gamma[c])
    }))
}

#[test]
fn roll_defaults_use_a_shared_gamma_of_point_six() {
    assert_eq!(DEFAULT_GAMMA, 0.60);
    let interpretation = neg(calibration(), DEFAULT_GAMMA, Balance::default());
    assert_eq!(interpretation.gamma, 0.60);
    assert_eq!(interpretation.channel_gammas().unwrap(), DVec3::splat(0.60));
}

#[test]
fn anchors_hold_for_all_supported_global_gammas_and_balances() {
    let cal = calibration();
    for gamma in [0.1, 0.3, DEFAULT_GAMMA, 0.65, 1.0, 3.0] {
        for balance in [
            Balance::default(),
            Balance {
                warmth: -1.0,
                tint: 1.0,
            },
            Balance {
                warmth: 0.7,
                tint: -0.4,
            },
        ] {
            let inversion = Inversion::new(neg(cal, gamma, balance)).unwrap();
            close_rgb(inversion.apply(cal.dmin).unwrap().rgb, DVec3::ZERO);
            close_rgb(inversion.apply(cal.dmax).unwrap().rgb, DVec3::ONE);
        }
    }
}

#[test]
fn matches_direct_formula_in_the_well_conditioned_domain() {
    let cal = calibration();
    let gamma = DEFAULT_GAMMA;
    let inversion = Inversion::new(neg(cal, gamma, Balance::default())).unwrap();
    for scan in [
        DVec3::new(0.05, 0.06, 0.04),
        DVec3::new(0.9, 0.55, 0.3),
        DVec3::splat(0.001),
    ] {
        let expected = DVec3::from_array(std::array::from_fn(|c| {
            ((cal.dmin[c] / scan[c]).powf(1.0 / gamma) - 1.0)
                / ((cal.dmin[c] / cal.dmax[c]).powf(1.0 / gamma) - 1.0)
        }));
        close_rgb(inversion.apply(scan).unwrap().rgb, expected);
    }
}

#[test]
fn preserves_negative_outputs_and_values_above_one() {
    let cal = calibration();
    let inversion = Inversion::new(neg(cal, DEFAULT_GAMMA, Balance::default())).unwrap();
    assert!(inversion.apply(cal.dmin * 1.1).unwrap().rgb.max_element() < 0.0);
    assert!(inversion.apply(cal.dmax / 2.0).unwrap().rgb.min_element() > 1.0);
}

#[test]
fn floors_only_scan_inputs_and_reports_each_affected_channel() {
    let cal = calibration();
    let inversion = Inversion::new(neg(cal, DEFAULT_GAMMA, Balance::default())).unwrap();
    let result = inversion.apply(DVec3::new(0.0, -0.01, 0.02)).unwrap();
    assert_eq!(result.floored, [true, true, false]);
    close_rgb(
        result.rgb,
        inversion
            .apply(DVec3::new(cal.input_floor, cal.input_floor, 0.02))
            .unwrap()
            .rgb,
    );
    assert!(matches!(
        inversion.apply(DVec3::new(f64::NAN, 0.1, 0.1)),
        Err(Error::NonFinitePixel { channel: 0 })
    ));
    assert!(inversion.apply(DVec3::splat(f64::INFINITY)).is_err());
}

#[test]
fn invalid_references_and_parameters_are_not_repaired() {
    let cal = calibration();
    for invalid in [
        Calibration {
            dmin: DVec3::ZERO,
            ..cal
        },
        Calibration {
            dmax: DVec3::ZERO,
            ..cal
        },
        Calibration {
            dmax: DVec3::splat(f64::NAN),
            ..cal
        },
        Calibration {
            dmin: cal.dmax,
            ..cal
        },
        Calibration {
            dmax: DVec3::splat(1e-8),
            ..cal
        },
        Calibration {
            dmax: cal.dmin * (1.0 - 1e-10),
            ..cal
        },
        Calibration {
            input_floor: 0.0,
            ..cal
        },
    ] {
        assert!(invalid.validate().is_err());
    }
    for gamma in [0.0, -1.0, f64::NAN, 3.1] {
        assert!(Inversion::new(neg(cal, gamma, Balance::default())).is_err());
    }
    assert!(exposure_multiplier(f64::INFINITY).is_err());
    assert!(
        wb_matrix(Balance {
            warmth: 1.1,
            tint: 0.0
        })
        .is_err()
    );
}

#[test]
fn ratio_evaluation_avoids_overflow_when_the_result_is_finite() {
    let cal = Calibration {
        dmin: DVec3::splat(1e200),
        dmax: DVec3::splat(1e-200),
        input_floor: 1e-250,
    };
    let inversion = Inversion::new(neg(cal, 1.0, Balance::default())).unwrap();
    close_rgb(inversion.apply(cal.dmax).unwrap().rgb, DVec3::ONE);
    close_rgb(
        inversion.apply(DVec3::splat(1e-201)).unwrap().rgb,
        DVec3::splat(10.0),
    );
    assert!(
        Inversion::new(neg(cal, 0.1, Balance::default()))
            .unwrap()
            .apply(DVec3::splat(1e-250))
            .is_err()
    );
}

#[test]
fn near_dmin_is_not_rounded_to_zero_by_power_subtraction() {
    let cal = calibration();
    let inversion = Inversion::new(neg(cal, DEFAULT_GAMMA, Balance::default())).unwrap();
    let result = inversion.apply(cal.dmin * (1.0 - 1e-12)).unwrap();
    assert!(result.rgb.min_element() > 0.0 && result.rgb.max_element() < 1e-10);
}

#[test]
fn exposure_scopes_combine_without_changing_inversion() {
    let cal = calibration();
    let scan = DVec3::new(0.06, 0.08, 0.04);
    let base = InputPipeline::new(
        inp(cal, DEFAULT_GAMMA, Balance::default()),
        RollSettings::default(),
        ap0_to_ap1(),
    )
    .unwrap()
    .apply(scan)
    .unwrap();
    let combined = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: -0.5,
            ..AdjustmentLayerSettings::default()
        }),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            inp(cal, DEFAULT_GAMMA, Balance::default()),
            RollSettings {
                exposure: 2.0,
                ..RollSettings::default()
            },
            layers,
            ap0_to_ap1(),
        )
    })
    .unwrap()
    .apply(scan, [0, 0])
    .unwrap();
    close_rgb(combined.rgb, base.rgb * 1.5_f64.exp2());
    assert_eq!(combined.floored, base.floored);
}

#[test]
fn gamma_balance_preserves_the_geometric_mean_global_gamma() {
    let interpretation = neg(
        calibration(),
        DEFAULT_GAMMA,
        Balance {
            warmth: 0.7,
            tint: -0.3,
        },
    );
    let gammas = interpretation.channel_gammas().unwrap();
    close(gammas.x * gammas.y * gammas.z, interpretation.gamma.powi(3));
}

#[test]
fn color_matrix_inverse_and_wb_identity() {
    let cones = ap1_to_cones();
    let product = cones.inverse() * cones;
    for column in 0..3 {
        close_rgb(product.col(column), DMat3::IDENTITY.col(column));
    }
    assert_eq!(wb_matrix(Balance::default()).unwrap(), DMat3::IDENTITY);
    close_rgb(
        AP0_TO_XYZ * DVec3::ONE,
        DVec3::new(0.9526460745698463, 1.0, 1.0088251843515859),
    );
}

#[test]
fn wb_gain_normalization_preserves_matrix_determinant() {
    close(
        wb_matrix(Balance {
            warmth: 0.6,
            tint: -0.4,
        })
        .unwrap()
        .determinant(),
        1.0,
    );
}

#[test]
fn wb_pick_moves_the_controls_to_the_actual_correction() {
    let desired = Balance {
        warmth: 0.24,
        tint: -0.17,
    };
    let cast = wb_matrix(desired).unwrap().inverse() * DVec3::splat(0.2);
    let solved = neutral_wb(cast).unwrap();
    close(solved.warmth, desired.warmth);
    close(solved.tint, desired.tint);
    close_rgb(wb_matrix(solved).unwrap() * cast, DVec3::splat(0.2));
    assert!(neutral_wb(DVec3::ZERO).is_err());
    assert!(neutral_wb(DVec3::splat(-0.1)).is_err());
    assert!(neutral_wb(DVec3::splat(f64::NAN)).is_err());
}

#[test]
fn photo_pick_uses_linear_patch_mean_and_does_not_change_shared_state() {
    let roll = RollSettings::default();
    let desired = Balance {
        warmth: -0.2,
        tint: 0.13,
    };
    let cast_matrix = wb_matrix(desired).unwrap().inverse();
    let patch = [0.04, 0.2, 1.5].map(|v| cast_matrix * DVec3::splat(v));
    let solved = white_balance(&patch).unwrap();
    close(solved.warmth, desired.warmth);
    close(solved.tint, desired.tint);
    assert_eq!(roll, RollSettings::default());
    assert!(white_balance(&[]).is_err());
}

#[test]
fn roll_pick_solves_the_mean_of_inverted_full_resolution_pixels() {
    let cal = calibration();
    let expected = Balance {
        warmth: 0.21,
        tint: -0.14,
    };
    let correct = neg(cal, DEFAULT_GAMMA, expected);
    let patch = [0.025, 0.09, 0.4].map(|v| scan_for_positive(cal, correct, DVec3::splat(v)));
    let initial = neg(
        cal,
        DEFAULT_GAMMA,
        Balance {
            warmth: -0.12,
            tint: 0.11,
        },
    );
    let solved = roll_neutral(initial, &patch, DMat3::IDENTITY).unwrap();
    assert!((solved.warmth - expected.warmth).abs() < 1e-8);
    assert!((solved.tint - expected.tint).abs() < 1e-8);
    assert_eq!(initial.gamma, correct.gamma);
    let inversion = Inversion::new(correct).unwrap();
    let inverted: Vec<_> = patch
        .into_iter()
        .map(|p| inversion.apply(p).unwrap().rgb)
        .collect();
    close_rgb(mean(&inverted).unwrap(), DVec3::splat(0.515 / 3.0));
    let inverted_mean_scan = inversion.apply(mean(&patch).unwrap()).unwrap().rgb;
    assert!((inverted_mean_scan.x - inverted_mean_scan.y).abs() > 1e-4);
}

#[test]
fn roll_pick_rejects_both_anchors_and_unusable_inputs_without_mutation() {
    let cal = calibration();
    let interpretation = neg(cal, DEFAULT_GAMMA, Balance::default());
    for patch in [
        vec![cal.dmin],
        vec![cal.dmax],
        vec![DVec3::ZERO],
        vec![DVec3::splat(f64::NAN)],
        vec![],
    ] {
        assert!(
            roll_neutral(interpretation, &patch, DMat3::IDENTITY).is_err(),
            "accepted {patch:?}"
        );
    }
    assert_eq!(cal, calibration());
}

#[test]
fn ap0_ap1_conversion_preserves_xyz_white_and_signed_hdr_values() {
    let forward = ap0_to_ap1();
    let reverse = ap1_to_ap0();
    close_rgb(AP1_TO_XYZ * DVec3::ONE, AP0_TO_XYZ * DVec3::ONE);
    close_rgb(forward * DVec3::ONE, DVec3::ONE);
    for ap0 in [
        DVec3::ONE,
        DVec3::new(-0.12, 0.7, 2.0),
        DVec3::new(0.0, 1.0, 0.0),
    ] {
        let ap1 = forward * ap0;
        close_rgb(AP1_TO_XYZ * ap1, AP0_TO_XYZ * ap0);
        close_rgb(reverse * ap1, ap0);
    }
    assert!((forward * DVec3::Y).min_element() < 0.0);
}

#[test]
fn pipeline_maps_camera_positive_to_ap1_and_ap0_export_preserves_xyz() {
    let cal = calibration();
    let interpretation = neg(cal, DEFAULT_GAMMA, Balance::default());
    let original = DVec3::new(4.0, 0.03, 0.1);
    let scan = scan_for_positive(cal, interpretation, original);
    let inverted = Inversion::new(interpretation).unwrap().apply(scan).unwrap();
    let processed = InputPipeline::new(
        inp(cal, DEFAULT_GAMMA, Balance::default()),
        RollSettings::default(),
        ap0_to_ap1(),
    )
    .unwrap()
    .apply(scan)
    .unwrap();
    close_rgb(inverted.rgb, original);
    close_rgb(processed.rgb, ap0_to_ap1() * inverted.rgb);
    assert!(processed.rgb.min_element() < 0.0);
    assert!(processed.rgb.max_element() > 1.0);
    close_rgb(ap1_to_ap0() * processed.rgb, inverted.rgb);
    assert_eq!(processed.floored, inverted.floored);
}

#[test]
fn moving_bradford_wb_to_ap1_preserves_the_xyz_operation() {
    let cal = calibration();
    let roll = RollSettings {
        exposure: 1.25,
        ..RollSettings::default()
    };
    let photo = AdjustmentLayerSettings {
        exposure: -0.75,
        wb: Balance {
            warmth: 0.3,
            tint: -0.2,
        },
        ..AdjustmentLayerSettings::default()
    };
    let scan = DVec3::new(0.05, 0.07, 0.02);
    let interpretation = neg(cal, DEFAULT_GAMMA, Balance::default());
    let inverted = Inversion::new(interpretation).unwrap().apply(scan).unwrap();
    let processed = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(photo),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            inp(cal, DEFAULT_GAMMA, Balance::default()),
            roll,
            layers,
            ap0_to_ap1(),
        )
    })
    .unwrap()
    .apply(scan, [0, 0])
    .unwrap();
    let previous_cones = BRADFORD * AP0_TO_XYZ;
    let previous_wb = previous_cones.inverse()
        * DMat3::from_diagonal(photo.wb.offsets().unwrap().exp())
        * previous_cones;
    let previous_ap0 = previous_wb * inverted.rgb * 0.5_f64.exp2();
    close_rgb(AP1_TO_XYZ * processed.rgb, AP0_TO_XYZ * previous_ap0);
    close_rgb(ap1_to_ap0() * processed.rgb, previous_ap0);
}

#[test]
fn white_balance_accepts_outside_ap1_rgb_when_cone_responses_are_usable() {
    let mut outside_ap1 = 0;
    for warmth in [-0.99, 0.0, 0.99] {
        for tint in [-0.99, 0.0, 0.99] {
            let correction = Balance { warmth, tint };
            let cast = wb_matrix(correction).unwrap().inverse() * DVec3::splat(0.2);
            outside_ap1 += usize::from(cast.min_element() < 0.0);
            let solved = white_balance(&[cast]).unwrap();
            close(solved.warmth, warmth);
            close(solved.tint, tint);
            close_rgb(wb_matrix(solved).unwrap() * cast, DVec3::splat(0.2));
        }
    }
    assert!(outside_ap1 > 0);
}

#[test]
fn photo_pick_uses_ap1_after_the_inversion_conversion() {
    let cal = calibration();
    let interpretation = neg(cal, DEFAULT_GAMMA, Balance::default());
    let correction = Balance {
        warmth: 0.24,
        tint: -0.17,
    };
    let cast_ap1 = wb_matrix(correction).unwrap().inverse() * DVec3::splat(0.2);
    let scan = scan_for_positive(cal, interpretation, ap1_to_ap0() * cast_ap1);
    let before_wb = InputPipeline::new(
        inp(cal, DEFAULT_GAMMA, Balance::default()),
        RollSettings::default(),
        ap0_to_ap1(),
    )
    .unwrap()
    .apply(scan)
    .unwrap();
    let solved = white_balance(&[before_wb.rgb]).unwrap();
    close(solved.warmth, correction.warmth);
    close(solved.tint, correction.tint);
    let positive = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
            wb: solved,
            ..AdjustmentLayerSettings::default()
        }),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            inp(cal, DEFAULT_GAMMA, Balance::default()),
            RollSettings::default(),
            layers,
            ap0_to_ap1(),
        )
    })
    .unwrap()
    .apply(scan, [0, 0])
    .unwrap();
    close_rgb(positive.rgb, DVec3::splat(0.2));
}

#[test]
fn patch_mean_preserves_signed_hdr_channels() {
    let pixels = [
        DVec3::new(4., -1., 0.5),
        DVec3::new(2., 0.5, -1.5),
        DVec3::new(-3., 2., 4.),
        DVec3::new(1., -0.5, 1.),
    ];
    assert_eq!(mean(&pixels).unwrap(), DVec3::new(1., 0.25, 1.));
}

#[test]
fn weighted_patch_mean_normalizes_fractional_weights_without_clipping() {
    let pixels = [
        (DVec3::new(4., -2., 0.5), 0.5),
        (DVec3::new(-2., 6., 4.5), 1.5),
    ];
    assert_eq!(weighted_mean(&pixels).unwrap(), DVec3::new(-0.5, 4., 3.5));
}

#[test]
fn patch_means_reject_empty_inputs() {
    assert!(matches!(mean(&[]), Err(Error::UnusableSample(_))));
    assert!(matches!(weighted_mean(&[]), Err(Error::UnusableSample(_))));
}

#[test]
fn patch_means_reject_nonfinite_pixels_in_each_channel() {
    for channel in 0..3 {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut pixel = DVec3::ONE;
            pixel[channel] = value;
            assert_eq!(mean(&[pixel]), Err(Error::NonFinitePixel { channel }));
            assert_eq!(
                weighted_mean(&[(pixel, 1.)]),
                Err(Error::NonFinitePixel { channel })
            );
        }
    }
}

#[test]
fn patch_means_reject_overflow() {
    let pixel = DVec3::splat(f64::MAX);
    assert_eq!(mean(&[pixel, pixel]), Err(Error::NumericalOverflow));
    assert_eq!(
        weighted_mean(&[(pixel, 1.), (pixel, 1.)]),
        Err(Error::NumericalOverflow)
    );
}

#[test]
fn weighted_patch_mean_rejects_nonpositive_or_nonfinite_weights() {
    for weight in [0., -1., f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            weighted_mean(&[(DVec3::ONE, weight)]),
            Err(Error::UnusableSample(_))
        ));
    }
}

#[test]
fn weighted_patch_mean_propagates_sample_read_failure() {
    let samples = [
        Ok((DVec3::ONE, 1.)),
        Err(Error::InvalidParameter("sample read failed")),
    ];
    assert_eq!(
        weighted_mean_iter(samples),
        Err(Error::InvalidParameter("sample read failed"))
    );
}

#[test]
fn layer_white_balance_fits_constant_and_varying_coverage_including_partial_strength() {
    use antitone::sampling::adjustment_layer_white_balance;
    let expected = Balance {
        warmth: 0.24,
        tint: -0.16,
    };
    for weights in [
        vec![1.; 4],
        vec![0.35; 4],
        vec![1e-5; 4],
        vec![1e-6; 4],
        vec![0., 0.1, 0.35, 0.7],
        vec![0.8, 0.6, 0.4, 0.2],
    ] {
        let pixels: Vec<_> = weights
            .iter()
            .enumerate()
            .map(|(i, &weight)| {
                let cast = Balance {
                    warmth: -weight * expected.warmth,
                    tint: -weight * expected.tint,
                };
                (
                    wb_matrix(cast).unwrap() * DVec3::splat(0.15 + i as f64 * 0.08),
                    0.25 + i as f64,
                    weight,
                )
            })
            .collect();
        let actual = adjustment_layer_white_balance(&pixels).unwrap();
        assert!(
            (actual.warmth - expected.warmth).abs() < 1e-8,
            "{weights:?}: {actual:?}"
        );
        assert!(
            (actual.tint - expected.tint).abs() < 1e-8,
            "{weights:?}: {actual:?}"
        );
        let corrected: Vec<_> = pixels
            .iter()
            .map(|&(rgb, area, weight)| {
                (
                    wb_matrix(Balance {
                        warmth: weight * actual.warmth,
                        tint: weight * actual.tint,
                    })
                    .unwrap()
                        * rgb,
                    area,
                )
            })
            .collect();
        let mean = weighted_mean(&corrected).unwrap();
        assert!((mean - DVec3::splat(mean.y)).abs().max_element() < 1e-9);
    }
}

#[test]
fn layer_white_balance_rejects_uncovered_invalid_and_unreachable_patches() {
    use antitone::sampling::adjustment_layer_white_balance;
    assert!(adjustment_layer_white_balance(&[]).is_err());
    assert!(adjustment_layer_white_balance(&[(DVec3::ONE, 1., 0.)]).is_err());
    for weight in [f64::NAN, -0.1, 1.1] {
        assert!(adjustment_layer_white_balance(&[(DVec3::ONE, 1., weight)]).is_err());
    }
    let cast = wb_matrix(Balance {
        warmth: -0.5,
        tint: 0.,
    })
    .unwrap()
        * DVec3::ONE;
    assert!(
        adjustment_layer_white_balance(&[(cast, 1., 0.1)]).is_err(),
        "a correction outside the layer's control range must not be silently clamped"
    );
}

#[test]
fn layer_white_balance_respects_fractional_patch_area_when_samples_disagree() {
    use antitone::sampling::adjustment_layer_white_balance;
    let a = wb_matrix(Balance {
        warmth: -0.1,
        tint: 0.05,
    })
    .unwrap()
        * DVec3::splat(0.25);
    let b = wb_matrix(Balance {
        warmth: 0.08,
        tint: -0.1,
    })
    .unwrap()
        * DVec3::splat(0.3);
    let expected_full_strength = neutral_wb(a * 0.25 + b * 0.75).unwrap();
    let actual = adjustment_layer_white_balance(&[(a, 0.25, 0.4), (b, 0.75, 0.4)]).unwrap();
    assert!((actual.warmth - expected_full_strength.warmth / 0.4).abs() < 1e-8);
    assert!((actual.tint - expected_full_strength.tint / 0.4).abs() < 1e-8);
    let repeated =
        adjustment_layer_white_balance(&[(a, 1., 0.4), (b, 1., 0.4), (b, 1., 0.4), (b, 1., 0.4)])
            .unwrap();
    assert!((actual.warmth - repeated.warmth).abs() < 1e-8);
    assert!((actual.tint - repeated.tint).abs() < 1e-8);
    let unweighted = adjustment_layer_white_balance(&[(a, 1., 0.4), (b, 1., 0.4)]).unwrap();
    assert!((actual.warmth - unweighted.warmth).abs() > 0.01);
    assert_eq!(
        adjustment_layer_white_balance(&[(DVec3::ONE, 1., 1e-5)]).unwrap(),
        Balance::default()
    );
}
