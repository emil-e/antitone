use antitone::Error;
use antitone::color::{
    AP0_TO_XYZ, AP1_TO_XYZ, BRADFORD, Balance, ap0_to_ap1, ap1_to_ap0, ap1_to_cones, neutral_wb,
    wb_matrix,
};
use antitone::processing::{
    Calibration, DEFAULT_GAMMA, Inversion, PhotoSettings, Pipeline, RollSettings,
    exposure_multiplier,
};
use antitone::sampling::{mean, photo_neutral, roll_neutral};

#[test]
fn weighted_neutral_picks_match_repeated_full_resolution_pixels_not_bounding_box_means() {
    use antitone::sampling::{photo_neutral_weighted, roll_neutral_weighted, weighted_mean};
    let cal = calibration();
    let correct = RollSettings {
        balance: Balance {
            warmth: 0.21,
            tint: -0.14,
        },
        ..RollSettings::default()
    };
    let matrix = DMat3::from_diagonal(DVec3::new(0.8, 1.1, 1.3));
    let target = matrix.inverse() * DVec3::ONE;
    let patch = [0.025, 0.09, 0.4].map(|v| scan_for_positive(cal, correct, target * v));
    let weighted = [(patch[0], 0.5), (patch[1], 1.), (patch[2], 0.5)];
    let repeated = [patch[0], patch[1], patch[1], patch[2]];
    assert!((weighted_mean(&weighted).unwrap() - mean(&repeated).unwrap()).length() < 1e-15);
    let a = photo_neutral_weighted(&weighted).unwrap();
    let b = photo_neutral(&repeated).unwrap();
    assert!((a.warmth - b.warmth).abs() < 1e-14 && (a.tint - b.tint).abs() < 1e-14);
    let roll = RollSettings::default();
    let a = roll_neutral_weighted(cal, roll, &weighted, matrix).unwrap();
    let b = roll_neutral(cal, roll, &repeated, matrix).unwrap();
    assert!((a.warmth - b.warmth).abs() < 1e-9 && (a.tint - b.tint).abs() < 1e-9);
    assert!(
        (a.warmth - correct.balance.warmth).abs() < 1e-8
            && (a.tint - correct.balance.tint).abs() < 1e-8
    );
    assert!(weighted_mean(&[(patch[0], 0.)]).is_err());
    assert!(weighted_mean(&[(patch[0], f64::NAN)]).is_err());
}
use glam::{DMat3, DVec3};

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

fn scan_for_positive(calibration: Calibration, roll: RollSettings, value: DVec3) -> DVec3 {
    let gamma = roll.channel_gammas().unwrap();
    DVec3::from_array(std::array::from_fn(|c| {
        let denominator = (calibration.dmin[c] / calibration.dmax[c]).powf(1.0 / gamma[c]) - 1.0;
        calibration.dmin[c] / (1.0 + value[c] * denominator).powf(gamma[c])
    }))
}

#[test]
fn roll_defaults_use_a_shared_gamma_of_point_six() {
    assert_eq!(DEFAULT_GAMMA, 0.60);
    assert_eq!(RollSettings::default().gamma, 0.60);
    let grade = RollSettings::new_grade();
    assert_eq!(grade.gamma, 0.60);
    assert_eq!(grade.channel_gammas().unwrap(), DVec3::splat(0.60));
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
            let inversion = Inversion::new(
                cal,
                RollSettings {
                    gamma,
                    balance,
                    exposure: 0.0,
                    ..RollSettings::default()
                },
            )
            .unwrap();
            close_rgb(inversion.apply(cal.dmin).unwrap().rgb, DVec3::ZERO);
            close_rgb(inversion.apply(cal.dmax).unwrap().rgb, DVec3::ONE);
        }
    }
}

#[test]
fn matches_direct_formula_in_the_well_conditioned_domain() {
    let cal = calibration();
    let roll = RollSettings::default();
    let inversion = Inversion::new(cal, roll).unwrap();
    for scan in [
        DVec3::new(0.05, 0.06, 0.04),
        DVec3::new(0.9, 0.55, 0.3),
        DVec3::splat(0.001),
    ] {
        let expected = DVec3::from_array(std::array::from_fn(|c| {
            ((cal.dmin[c] / scan[c]).powf(1.0 / roll.gamma) - 1.0)
                / ((cal.dmin[c] / cal.dmax[c]).powf(1.0 / roll.gamma) - 1.0)
        }));
        close_rgb(inversion.apply(scan).unwrap().rgb, expected);
    }
}

#[test]
fn preserves_negative_outputs_and_values_above_one() {
    let cal = calibration();
    let inversion = Inversion::new(cal, RollSettings::default()).unwrap();
    assert!(inversion.apply(cal.dmin * 1.1).unwrap().rgb.max_element() < 0.0);
    assert!(inversion.apply(cal.dmax / 2.0).unwrap().rgb.min_element() > 1.0);
}

#[test]
fn floors_only_scan_inputs_and_reports_each_affected_channel() {
    let cal = calibration();
    let inversion = Inversion::new(cal, RollSettings::default()).unwrap();
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
        assert!(
            Inversion::new(
                cal,
                RollSettings {
                    gamma,
                    ..RollSettings::default()
                }
            )
            .is_err()
        );
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
    let inversion = Inversion::new(
        cal,
        RollSettings {
            gamma: 1.0,
            ..RollSettings::default()
        },
    )
    .unwrap();
    close_rgb(inversion.apply(cal.dmax).unwrap().rgb, DVec3::ONE);
    close_rgb(
        inversion.apply(DVec3::splat(1e-201)).unwrap().rgb,
        DVec3::splat(10.0),
    );
    assert!(
        Inversion::new(
            cal,
            RollSettings {
                gamma: 0.1,
                ..RollSettings::default()
            }
        )
        .unwrap()
        .apply(DVec3::splat(1e-250))
        .is_err()
    );
}

#[test]
fn near_dmin_is_not_rounded_to_zero_by_power_subtraction() {
    let cal = calibration();
    let inversion = Inversion::new(cal, RollSettings::default()).unwrap();
    let result = inversion.apply(cal.dmin * (1.0 - 1e-12)).unwrap();
    assert!(result.rgb.min_element() > 0.0 && result.rgb.max_element() < 1e-10);
}

#[test]
fn exposure_scopes_combine_without_changing_inversion() {
    let cal = calibration();
    let scan = DVec3::new(0.06, 0.08, 0.04);
    let roll = RollSettings::default();
    let base = Pipeline::new(cal, roll, PhotoSettings::default(), ap0_to_ap1())
        .unwrap()
        .apply(scan)
        .unwrap();
    let combined = Pipeline::new(
        cal,
        RollSettings {
            exposure: 2.0,
            ..roll
        },
        PhotoSettings {
            exposure: -0.5,
            ..PhotoSettings::default()
        },
        ap0_to_ap1(),
    )
    .unwrap()
    .apply(scan)
    .unwrap();
    close_rgb(combined.rgb, base.rgb * 1.5_f64.exp2());
    assert_eq!(combined.floored, base.floored);
}

#[test]
fn gamma_balance_preserves_the_geometric_mean_global_gamma() {
    let roll = RollSettings {
        balance: Balance {
            warmth: 0.7,
            tint: -0.3,
        },
        ..RollSettings::default()
    };
    let gammas = roll.channel_gammas().unwrap();
    close(gammas.x * gammas.y * gammas.z, roll.gamma.powi(3));
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
    let solved = photo_neutral(&patch).unwrap();
    close(solved.warmth, desired.warmth);
    close(solved.tint, desired.tint);
    assert_eq!(roll, RollSettings::default());
    assert!(photo_neutral(&[]).is_err());
}

#[test]
fn roll_pick_solves_the_mean_of_inverted_full_resolution_pixels() {
    let cal = calibration();
    let expected = Balance {
        warmth: 0.21,
        tint: -0.14,
    };
    let correct_roll = RollSettings {
        balance: expected,
        ..RollSettings::default()
    };
    let patch = [0.025, 0.09, 0.4].map(|v| scan_for_positive(cal, correct_roll, DVec3::splat(v)));
    let initial = RollSettings {
        exposure: 3.0,
        balance: Balance {
            warmth: -0.12,
            tint: 0.11,
        },
        ..RollSettings::default()
    };
    let solved = roll_neutral(cal, initial, &patch, DMat3::IDENTITY).unwrap();
    assert!((solved.warmth - expected.warmth).abs() < 1e-8);
    assert!((solved.tint - expected.tint).abs() < 1e-8);
    assert_eq!(initial.exposure, 3.0);
    assert_eq!(initial.gamma, correct_roll.gamma);
    let inversion = Inversion::new(cal, correct_roll).unwrap();
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
    let roll = RollSettings::default();
    for patch in [
        vec![cal.dmin],
        vec![cal.dmax],
        vec![DVec3::ZERO],
        vec![DVec3::splat(f64::NAN)],
        vec![],
    ] {
        assert!(
            roll_neutral(cal, roll, &patch, DMat3::IDENTITY).is_err(),
            "accepted {patch:?}"
        );
    }
    assert_eq!(roll, RollSettings::default());
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
    let roll = RollSettings::default();
    let original = DVec3::new(4.0, 0.03, 0.1);
    let scan = scan_for_positive(cal, roll, original);
    let inverted = Inversion::new(cal, roll).unwrap().apply(scan).unwrap();
    let processed = Pipeline::new(cal, roll, PhotoSettings::default(), ap0_to_ap1())
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
    let photo = PhotoSettings {
        exposure: -0.75,
        wb: Balance {
            warmth: 0.3,
            tint: -0.2,
        },
        ..PhotoSettings::default()
    };
    let scan = DVec3::new(0.05, 0.07, 0.02);
    let inverted = Inversion::new(cal, roll).unwrap().apply(scan).unwrap();
    let processed = Pipeline::new(cal, roll, photo, ap0_to_ap1())
        .unwrap()
        .apply(scan)
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
fn photo_neutral_accepts_outside_ap1_rgb_when_cone_responses_are_usable() {
    let mut outside_ap1 = 0;
    for warmth in [-0.99, 0.0, 0.99] {
        for tint in [-0.99, 0.0, 0.99] {
            let correction = Balance { warmth, tint };
            let cast = wb_matrix(correction).unwrap().inverse() * DVec3::splat(0.2);
            outside_ap1 += usize::from(cast.min_element() < 0.0);
            let solved = photo_neutral(&[cast]).unwrap();
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
    let roll = RollSettings::default();
    let correction = Balance {
        warmth: 0.24,
        tint: -0.17,
    };
    let cast_ap1 = wb_matrix(correction).unwrap().inverse() * DVec3::splat(0.2);
    let scan = scan_for_positive(cal, roll, ap1_to_ap0() * cast_ap1);
    let before_wb = Pipeline::new(cal, roll, PhotoSettings::default(), ap0_to_ap1())
        .unwrap()
        .apply(scan)
        .unwrap();
    let solved = photo_neutral(&[before_wb.rgb]).unwrap();
    close(solved.warmth, correction.warmth);
    close(solved.tint, correction.tint);
    let positive = Pipeline::new(
        cal,
        roll,
        PhotoSettings {
            wb: solved,
            ..PhotoSettings::default()
        },
        ap0_to_ap1(),
    )
    .unwrap()
    .apply(scan)
    .unwrap();
    close_rgb(positive.rgb, DVec3::splat(0.2));
}

#[test]
fn compensated_patch_mean_preserves_small_contributions() {
    close_rgb(
        mean(&[
            DVec3::splat(1e16),
            DVec3::ONE,
            DVec3::ONE,
            DVec3::splat(-1e16),
        ])
        .unwrap(),
        DVec3::splat(0.5),
    );
    assert!(mean(&[DVec3::splat(f64::NAN)]).is_err());
}
