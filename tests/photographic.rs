use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    photographic::Grade,
    processing::{Calibration, RollSettings},
};
use glam::{DMat3, DVec3};
#[path = "support/photographic_reference.rs"]
mod reference;

fn near(actual: DVec3, expected: DVec3) {
    for c in 0..3 {
        assert!(
            (actual[c] - expected[c]).abs() <= 2e-12 + 2e-11 * expected[c].abs(),
            "{actual:?} != {expected:?}"
        );
    }
}

fn inputs() -> Vec<DVec3> {
    let mut out = vec![DVec3::new(1., -0.4, 0.02), DVec3::new(4., 0.3, 0.02)];
    for value in [-1., -1e-12, 0., 1e-12, 0.0078125, 0.18, 1., 16., 1e10] {
        out.push(DVec3::splat(value));
        out.push(DVec3::new(value + 0.02, value - 0.01, value));
    }
    let v = DVec3::new(0.2, -0.1, 0.01);
    out.push(v - DVec3::splat(reference::y(v)));
    out
}

#[test]
fn default_blends_select_rgb_contrast_and_constant_y_saturation() {
    let defaults = AdjustmentLayerSettings::default();
    assert_eq!(defaults.contrast_colour_mix, 0.);
    assert_eq!(defaults.saturation_density, 0.);
    for photo in [
        AdjustmentLayerSettings {
            contrast: 1.2,
            ..defaults
        },
        AdjustmentLayerSettings {
            saturation: 1.3,
            ..defaults
        },
        AdjustmentLayerSettings {
            contrast: 1.2,
            saturation: 1.3,
            ..defaults
        },
    ] {
        let grade = Grade::new(photo).unwrap();
        for rgb in inputs() {
            let contrasted =
                reference::contrast(rgb, photo.contrast, photo.pivot, reference::Contrast::Rgb)
                    .unwrap();
            near(
                grade.apply(rgb).unwrap(),
                reference::saturation(
                    contrasted,
                    photo.saturation,
                    reference::Saturation::ConstantY,
                )
                .unwrap(),
            );
        }
    }
}

#[test]
fn production_matches_signed_hdr_reference_for_both_blends() {
    for contrast in [0.4, 0.5, 0.8, 1., 1.2, 1.5, 1.6] {
        for pivot in [0.01, 0.18, 1.] {
            for colour in [0., 0.5, 1.] {
                for saturation in [0., 0.7, 1., 1.3, 2.] {
                    for density in [0., 0.5, 1.] {
                        let photo = AdjustmentLayerSettings {
                            contrast,
                            pivot,
                            contrast_colour_mix: colour,
                            saturation,
                            saturation_density: density,
                            ..AdjustmentLayerSettings::default()
                        };
                        let grade = Grade::new(photo).unwrap();
                        for rgb in inputs() {
                            let c = reference::blend(
                                reference::contrast(rgb, contrast, pivot, reference::Contrast::Rgb)
                                    .unwrap(),
                                reference::contrast(
                                    rgb,
                                    contrast,
                                    pivot,
                                    reference::Contrast::BlackOffsetRatios,
                                )
                                .unwrap(),
                                colour,
                            )
                            .unwrap();
                            let expected = reference::blend(
                                reference::saturation(
                                    c,
                                    saturation,
                                    reference::Saturation::ConstantY,
                                )
                                .unwrap(),
                                reference::saturation(
                                    c,
                                    saturation,
                                    reference::Saturation::Maximum,
                                )
                                .unwrap(),
                                density,
                            )
                            .unwrap();
                            near(grade.apply(rgb).unwrap(), expected);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn decode_toe_crossings_do_not_create_a_near_zero_y_gain_jump() {
    let weights = antitone::color::AP1_TO_XYZ.row(1);
    let plane = DVec3::new(0.25, -weights.x / weights.y * 0.25, 0.);
    for contrast in [0.4_f64, 0.5, 0.8] {
        let qp = (0.155251141552511 - contrast * 0.0729055341958355) / (1. - contrast);
        let pivot = (qp * 17.52 - 9.72).exp2();
        for dp in [-1e-15, 0., 1e-15] {
            let grade = Grade::new(AdjustmentLayerSettings {
                contrast,
                pivot: pivot + dp,
                contrast_colour_mix: 1.,
                ..AdjustmentLayerSettings::default()
            })
            .unwrap();
            let black = grade.apply(DVec3::ZERO).unwrap();
            for dy in [-1e-16, 0., 1e-16] {
                let rgb = plane + DVec3::Y * dy;
                near(grade.apply(rgb).unwrap(), black + contrast * rgb);
                near(
                    reference::contrast(
                        rgb,
                        contrast,
                        pivot + dp,
                        reference::Contrast::BlackOffsetRatios,
                    )
                    .unwrap(),
                    black + contrast * rgb,
                );
            }
        }
    }
}

#[test]
fn amount_identity_is_exact_at_every_blend_and_pivot() {
    for pivot in [0.01, 0.18, 1.] {
        for colour in [0., 0.5, 1.] {
            for density in [0., 0.5, 1.] {
                let grade = Grade::new(AdjustmentLayerSettings {
                    pivot,
                    contrast_colour_mix: colour,
                    saturation_density: density,
                    ..AdjustmentLayerSettings::default()
                })
                .unwrap();
                for rgb in inputs() {
                    assert_eq!(grade.apply(rgb).unwrap(), rgb);
                }
            }
        }
    }
}

#[test]
fn grade_follows_exposure_wb_and_preserves_camera_floor_flags() {
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.02),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::default();
    let matrix = DMat3::from_cols(
        DVec3::new(1.2, -0.1, -0.1),
        DVec3::new(-0.1, 1.2, -0.1),
        DVec3::new(-0.1, -0.1, 1.2),
    );
    let base = AdjustmentLayerSettings {
        exposure: 1.2,
        wb: antitone::color::Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        ..AdjustmentLayerSettings::default()
    };
    let photo = AdjustmentLayerSettings {
        contrast: 1.2,
        saturation: 1.3,
        ..base
    };
    let original = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(base),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            input_for(calibration),
            roll,
            layers,
            matrix,
        )
    })
    .unwrap();
    let pipeline = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(photo),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            input_for(calibration),
            roll,
            layers,
            matrix,
        )
    })
    .unwrap();
    let grade = Grade::new(photo).unwrap();
    for scan in [DVec3::new(0., 0.05, 0.03), DVec3::new(0.3, 0.1, 0.05)] {
        let before = original.apply(scan, [0, 0]).unwrap();
        let after = pipeline.apply(scan, [0, 0]).unwrap();
        near(after.rgb, grade.apply(before.rgb).unwrap());
        assert_eq!(after.floored, before.floored);
    }
}

#[test]
fn invalid_parameters_and_overflow_reject_without_clamps() {
    for bad in [0.4 - f64::EPSILON, 1.6 + f64::EPSILON] {
        assert!(
            Grade::new(AdjustmentLayerSettings {
                contrast: bad,
                ..AdjustmentLayerSettings::default()
            })
            .is_err()
        );
    }
    for bad in [f64::NAN, f64::INFINITY, -1., 3.] {
        let base = AdjustmentLayerSettings::default();
        for photo in [
            AdjustmentLayerSettings {
                contrast: bad,
                ..base
            },
            AdjustmentLayerSettings { pivot: bad, ..base },
            AdjustmentLayerSettings {
                contrast_colour_mix: bad,
                ..base
            },
            AdjustmentLayerSettings {
                saturation: bad,
                ..base
            },
            AdjustmentLayerSettings {
                saturation_density: bad,
                ..base
            },
        ] {
            assert!(photo.validate().is_err());
        }
    }
    assert!(
        Grade::new(AdjustmentLayerSettings {
            contrast: 1.5,
            ..AdjustmentLayerSettings::default()
        })
        .unwrap()
        .apply(DVec3::splat(f64::MAX))
        .is_err()
    );
    assert!(
        Grade::new(AdjustmentLayerSettings::default())
            .unwrap()
            .apply(DVec3::splat(f64::NAN))
            .is_err()
    );
}

#[cfg(feature = "project")]
#[test]
fn current_photo_json_defaults_old_fields_and_retains_new_fields_strictly() {
    let old = r#"{"exposure":1.25,"wb":{"warmth":0.1,"tint":-0.2}}"#;
    let loaded: AdjustmentLayerSettings = serde_json::from_str(old).unwrap();
    assert_eq!(loaded.contrast_colour_mix, 0.);
    assert_eq!(loaded.saturation_density, 0.);
    assert_eq!(
        loaded,
        AdjustmentLayerSettings {
            exposure: 1.25,
            wb: antitone::color::Balance {
                warmth: 0.1,
                tint: -0.2
            },
            ..AdjustmentLayerSettings::default()
        }
    );
    let photo = AdjustmentLayerSettings {
        contrast: 1.123456789012345,
        pivot: 0.123456789012345,
        contrast_colour_mix: 0.23456789012345,
        saturation: 1.23456789012345,
        saturation_density: 0.3456789012345,
        ..loaded
    };
    let json = serde_json::to_string(&photo).unwrap();
    assert_eq!(
        serde_json::from_str::<AdjustmentLayerSettings>(&json).unwrap(),
        photo
    );
    let halfway = AdjustmentLayerSettings {
        contrast_colour_mix: 0.5,
        saturation_density: 0.5,
        ..photo
    };
    let json = serde_json::to_string(&halfway).unwrap();
    assert_eq!(
        serde_json::from_str::<AdjustmentLayerSettings>(&json).unwrap(),
        halfway
    );
    assert!(
        serde_json::from_str::<AdjustmentLayerSettings>(
            &old.replace("\"exposure\"", "\"unknown\"")
        )
        .is_err()
    );
}

fn input_for(cal: antitone::processing::Calibration) -> antitone::processing::InputSettings {
    antitone::processing::InputSettings::Negative {
        dmin: Some(cal.dmin),
        dmax: Some(cal.dmax),
        input_floor: cal.input_floor,
        response: antitone::processing::Response::film(),
        gamma: antitone::processing::DEFAULT_GAMMA,
        balance: antitone::color::Balance::default(),
    }
}
