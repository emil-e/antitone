use antitone::{
    Error,
    color::{AP1_TO_XYZ, BRADFORD, Balance, wb_matrix},
    crop::Crop,
    geometry::{
        Composition, Flips, PhotoOrientation, QuarterTurns, RollOrientation, SourcePoint,
        SourceRaster, SourceRegion,
    },
    local_adjustments::{GradientMask, LocalAdjustment, LocalGrade, weighted_settings},
    photographic::Grade,
    processing::{Calibration, InvertedPixel, PhotoSettings, Pipeline, RollSettings},
};
use glam::{DMat3, DVec3};

#[path = "support/photographic_reference.rs"]
mod reference;

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2e-12 + 2e-11 * expected.abs(),
        "{actual} != {expected}"
    );
}

fn near_rgb(actual: DVec3, expected: DVec3) {
    for c in 0..3 {
        near(actual[c], expected[c]);
    }
}

fn linear() -> GradientMask {
    GradientMask::Linear {
        start: SourcePoint([10., 20.]),
        end: SourcePoint([18., 20.]),
    }
}

fn adjustment(photo: PhotoSettings) -> LocalAdjustment {
    LocalAdjustment {
        mask: linear(),
        photo,
        enabled: true,
        invert: false,
        strength: 1.,
    }
}

fn pixel(rgb: DVec3) -> InvertedPixel {
    InvertedPixel {
        rgb,
        floored: [true, false, true],
    }
}

fn inputs() -> Vec<DVec3> {
    let mut values = vec![DVec3::new(4., -0.4, 0.02)];
    for value in [-1., -1e-12, 0., 1e-12, 0.0078125, 0.18, 1., 16., 1e10] {
        values.push(DVec3::splat(value));
        values.push(DVec3::new(value + 0.02, value - 0.01, value));
    }
    let weights = AP1_TO_XYZ.row(1);
    values.push(DVec3::new(0.25, -weights.x / weights.y * 0.25, 0.));
    values
}

#[test]
fn linear_boundaries_midpoint_direction_and_perpendicular_coverage() {
    let mask = linear();
    for (x, expected) in [
        (-100., 0.),
        (10., 0.),
        (12., 0.15625),
        (14., 0.5),
        (16., 0.84375),
        (18., 1.),
        (100., 1.),
    ] {
        for y in [-30., 20., 900.] {
            assert_eq!(mask.coverage(SourcePoint([x, y])).unwrap(), expected);
        }
    }
    let reversed = GradientMask::Linear {
        start: SourcePoint([18., 20.]),
        end: SourcePoint([10., 20.]),
    };
    for x in [0., 10., 12., 14., 16., 18., 30.] {
        near(
            reversed.coverage(SourcePoint([x, 31.])).unwrap(),
            1. - mask.coverage(SourcePoint([x, 31.])).unwrap(),
        );
    }
    let diagonal = GradientMask::Linear {
        start: SourcePoint([-10., -20.]),
        end: SourcePoint([10., 20.]),
    };
    near(diagonal.coverage(SourcePoint([0., 0.])).unwrap(), 0.5);
    near(diagonal.coverage(SourcePoint([20., -10.])).unwrap(), 0.5);
}

#[test]
fn linear_coverage_is_bounded_and_monotonic_near_endpoints() {
    let mask = GradientMask::Linear {
        start: SourcePoint([0., 0.]),
        end: SourcePoint([1., 0.]),
    };
    let mut last = 0.;
    for x in (0..=10000).map(|i| i as f64 / 10000.) {
        let value = mask.coverage(SourcePoint([x, 0.])).unwrap();
        assert!((0. ..=1.).contains(&value));
        assert!(value >= last);
        last = value;
    }
    for x in [f64::EPSILON, 1e-12, 1. - 1e-12, 1. - f64::EPSILON] {
        let value = mask.coverage(SourcePoint([x, 0.])).unwrap();
        assert!((0. ..=1.).contains(&value));
        near(value, x * x * (3. - 2. * x));
    }
}

#[test]
fn radial_inner_outer_boundaries_and_feather_extremes() {
    for feather in [0., 0.25, 0.5, 1.] {
        let mask = GradientMask::Radial {
            center: SourcePoint([10., 20.]),
            radii: [8., 4.],
            angle: 0.,
            feather,
        };
        assert_eq!(mask.coverage(SourcePoint([10., 20.])).unwrap(), 1.);
        assert_eq!(mask.coverage(SourcePoint([18., 20.])).unwrap(), 0.);
        assert_eq!(mask.coverage(SourcePoint([10., 24.])).unwrap(), 0.);
        assert_eq!(mask.coverage(SourcePoint([-100., 20.])).unwrap(), 0.);
        if feather == 0. {
            assert_eq!(mask.coverage(SourcePoint([17.999, 20.])).unwrap(), 1.);
        } else {
            for (fraction, expected) in [(0., 1.), (0.25, 0.84375), (0.5, 0.5), (1., 0.)] {
                let radius = 1. - feather + fraction * feather;
                near(
                    mask.coverage(SourcePoint([10. + 8. * radius, 20.]))
                        .unwrap(),
                    expected,
                );
                near(
                    mask.coverage(SourcePoint([10., 20. - 4. * radius]))
                        .unwrap(),
                    expected,
                );
            }
        }
    }
}

#[test]
fn radial_rotation_retains_native_axis_lengths() {
    for angle in [0., 0.37, std::f64::consts::FRAC_PI_2, -1.1] {
        let mask = GradientMask::Radial {
            center: SourcePoint([110., 220.]),
            radii: [8., 2.],
            angle,
            feather: 1.,
        };
        let (sin, cos) = angle.sin_cos();
        for (x, y) in [(4., 0.), (0., 1.), (-4., 0.), (0., -1.)] {
            near(
                mask.coverage(SourcePoint([
                    110. + cos * x - sin * y,
                    220. + sin * x + cos * y,
                ]))
                .unwrap(),
                0.5,
            );
        }
        near(
            mask.coverage(SourcePoint([110. - sin * 3., 220. + cos * 3.]))
                .unwrap(),
            0.,
        );
    }
}

#[test]
fn weighted_controls_retain_auxiliary_values_and_exact_endpoints() {
    let photo = PhotoSettings {
        exposure: 2.,
        wb: Balance {
            warmth: 0.8,
            tint: -0.4,
        },
        contrast: 1.6,
        pivot: 0.11,
        contrast_colour_mix: 0.7,
        saturation: 0.,
        saturation_density: 0.3,
    };
    assert_eq!(weighted_settings(photo, 1.).unwrap(), photo);
    let half = weighted_settings(photo, 0.5).unwrap();
    assert_eq!(half.exposure, 1.);
    assert_eq!(
        half.wb,
        Balance {
            warmth: 0.4,
            tint: -0.2
        }
    );
    assert_eq!(half.contrast, 1.3);
    assert_eq!(half.saturation, 0.5);
    assert_eq!(half.pivot, photo.pivot);
    assert_eq!(half.contrast_colour_mix, photo.contrast_colour_mix);
    assert_eq!(half.saturation_density, photo.saturation_density);
    assert_eq!(
        weighted_settings(photo, 0.).unwrap(),
        PhotoSettings {
            pivot: photo.pivot,
            contrast_colour_mix: photo.contrast_colour_mix,
            saturation_density: photo.saturation_density,
            ..PhotoSettings::default()
        }
    );
    for contrast in [0.4, 1.6] {
        let photo = PhotoSettings { contrast, ..photo };
        assert_eq!(weighted_settings(photo, 1.).unwrap(), photo);
        for w in [0., 1e-12, 0.25, 0.5, 0.75, 1. - f64::EPSILON, 1.] {
            weighted_settings(photo, w).unwrap().validate().unwrap();
        }
    }
}

#[test]
fn exposure_feather_scales_ev_not_pixel_output() {
    let grade = LocalGrade::new(vec![adjustment(PhotoSettings {
        exposure: 1.,
        ..PhotoSettings::default()
    })])
    .unwrap();
    let input = pixel(DVec3::new(4., -0.5, 0.));
    let output = grade.apply(input, SourcePoint([14., 20.])).unwrap();
    near_rgb(output.rgb, input.rgb * 2_f64.sqrt());
    assert!((output.rgb.x - input.rgb.x * 1.5).abs() > 0.1);
    assert_eq!(output.floored, input.floored);
}

#[test]
fn invert_precedes_strength_for_linear_and_radial_masks() {
    for mask in [
        linear(),
        GradientMask::Radial {
            center: SourcePoint([10., 20.]),
            radii: [8., 4.],
            angle: 0.,
            feather: 1.,
        },
    ] {
        for invert in [false, true] {
            let grade = LocalGrade::new(vec![LocalAdjustment {
                mask,
                invert,
                strength: 0.25,
                ..adjustment(PhotoSettings {
                    exposure: 2.,
                    ..PhotoSettings::default()
                })
            }])
            .unwrap();
            for point in [
                SourcePoint([10., 20.]),
                SourcePoint([12., 20.]),
                SourcePoint([18., 20.]),
            ] {
                let coverage = mask.coverage(point).unwrap();
                let w = if invert { 1. - coverage } else { coverage } * 0.25;
                let input = pixel(DVec3::new(-0.1, 0.2, 4.));
                near_rgb(
                    grade.apply(input, point).unwrap().rgb,
                    input.rgb * (2. * w).exp2(),
                );
            }
        }
    }
}

#[test]
fn empty_disabled_zero_strength_zero_coverage_and_identity_stages_bypass_exactly() {
    let active = adjustment(PhotoSettings {
        exposure: 2.,
        ..PhotoSettings::default()
    });
    let identity = adjustment(PhotoSettings {
        pivot: 1.,
        contrast_colour_mix: 1.,
        saturation_density: 1.,
        ..PhotoSettings::default()
    });
    let point = SourcePoint([14., 20.]);
    for records in [
        vec![],
        vec![identity],
        vec![LocalAdjustment {
            enabled: false,
            ..active
        }],
        vec![LocalAdjustment {
            strength: 0.,
            ..active
        }],
    ] {
        let grade = LocalGrade::new(records).unwrap();
        for rgb in inputs().into_iter().chain([DVec3::new(-0., 0., -0.)]) {
            let input = pixel(rgb);
            let out = grade.apply(input, point).unwrap();
            assert_eq!(out, input);
            assert_eq!(
                out.rgb.to_array().map(f64::to_bits),
                input.rgb.to_array().map(f64::to_bits)
            );
        }
    }
    let grade = LocalGrade::new(vec![active]).unwrap();
    let input = pixel(DVec3::new(-0., 0., -0.));
    let out = grade.apply(input, SourcePoint([0., 20.])).unwrap();
    assert_eq!(
        out.rgb.to_array().map(f64::to_bits),
        input.rgb.to_array().map(f64::to_bits)
    );
}

#[test]
fn full_weight_matches_existing_photographic_operators() {
    let photo = PhotoSettings {
        exposure: 1.25,
        wb: Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        contrast: 0.4,
        pivot: 0.11,
        contrast_colour_mix: 0.7,
        saturation: 1.3,
        saturation_density: 0.6,
    };
    let grade = LocalGrade::new(vec![adjustment(photo)]).unwrap();
    let matrix = wb_matrix(photo.wb).unwrap() * photo.exposure.exp2();
    for rgb in inputs() {
        let output = grade.apply(pixel(rgb), SourcePoint([18., 20.])).unwrap();
        assert_eq!(
            output.rgb,
            Grade::new(photo).unwrap().apply(matrix * rgb).unwrap()
        );
        assert_eq!(output.floored, [true, false, true]);
    }
}

#[test]
fn fractional_signed_hdr_stages_match_independent_control_space_reference() {
    let cones = BRADFORD * AP1_TO_XYZ;
    for contrast in [0.4, 1., 1.6] {
        for saturation in [0., 1., 2.] {
            for blend in [0., 0.5, 1.] {
                let photo = PhotoSettings {
                    exposure: -1.5,
                    wb: Balance {
                        warmth: 0.6,
                        tint: -0.2,
                    },
                    contrast,
                    pivot: 0.11,
                    contrast_colour_mix: blend,
                    saturation,
                    saturation_density: blend,
                };
                let grade = LocalGrade::new(vec![adjustment(photo)]).unwrap();
                for (x, w) in [
                    (10., 0.),
                    (12., 0.15625),
                    (14., 0.5),
                    (16., 0.84375),
                    (18., 1.),
                ] {
                    let warmth = w * 0.6 / 2_f64.sqrt();
                    let tint = w * -0.2 / 6_f64.sqrt();
                    let gains = DVec3::new(warmth + tint, -2. * tint, -warmth + tint).exp();
                    for rgb in inputs() {
                        let wb = cones.inverse() * (gains * (cones * rgb));
                        let balanced = wb * (-1.5_f64 * w).exp2();
                        let a = 1. + w * (contrast - 1.);
                        let contrasted = reference::blend(
                            reference::contrast(balanced, a, 0.11, reference::Contrast::Rgb)
                                .unwrap(),
                            reference::contrast(
                                balanced,
                                a,
                                0.11,
                                reference::Contrast::BlackOffsetRatios,
                            )
                            .unwrap(),
                            blend,
                        )
                        .unwrap();
                        let s = 1. + w * (saturation - 1.);
                        let expected = reference::blend(
                            reference::saturation(contrasted, s, reference::Saturation::ConstantY)
                                .unwrap(),
                            reference::saturation(contrasted, s, reference::Saturation::Maximum)
                                .unwrap(),
                            blend,
                        )
                        .unwrap();
                        let output = grade.apply(pixel(rgb), SourcePoint([x, 20.])).unwrap();
                        near_rgb(output.rgb, expected);
                        assert_eq!(output.floored, [true, false, true]);
                    }
                }
            }
        }
    }
}

#[test]
fn consecutive_local_exposures_add_weighted_ev() {
    let first = adjustment(PhotoSettings {
        exposure: 2.,
        ..PhotoSettings::default()
    });
    let second = LocalAdjustment {
        invert: true,
        strength: 0.5,
        ..adjustment(PhotoSettings {
            exposure: -1.,
            ..PhotoSettings::default()
        })
    };
    let grade = LocalGrade::new(vec![first, second]).unwrap();
    for (x, coverage) in [
        (10., 0.),
        (12., 0.15625),
        (14., 0.5),
        (16., 0.84375),
        (18., 1.),
    ] {
        let ev: f64 = 2. * coverage - (1. - coverage) * 0.5;
        let input = pixel(DVec3::new(-0.2, 0.3, 16.));
        near_rgb(
            grade.apply(input, SourcePoint([x, 20.])).unwrap().rgb,
            input.rgb * ev.exp2(),
        );
    }
}

#[test]
fn exposure_and_contrast_stages_obey_list_order() {
    let exposure = adjustment(PhotoSettings {
        exposure: 1.,
        ..PhotoSettings::default()
    });
    let contrast = adjustment(PhotoSettings {
        contrast: 1.4,
        ..PhotoSettings::default()
    });
    let before = LocalGrade::new(vec![exposure, contrast]).unwrap();
    let after = LocalGrade::new(vec![contrast, exposure]).unwrap();
    let rgb = DVec3::new(0.02, 0.1, 0.4);
    let point = SourcePoint([18., 20.]);
    let a = before.apply(pixel(rgb), point).unwrap().rgb;
    let b = after.apply(pixel(rgb), point).unwrap().rgb;
    near_rgb(
        a,
        reference::contrast(rgb * 2., 1.4, 0.18, reference::Contrast::Rgb).unwrap(),
    );
    near_rgb(
        b,
        reference::contrast(rgb, 1.4, 0.18, reference::Contrast::Rgb).unwrap() * 2.,
    );
    assert!((a - b).abs().max_element() > 1e-3);
}

#[test]
fn locals_follow_unchanged_global_pipeline_and_preserve_floor_flags() {
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.02),
        input_floor: 1. / 65535.,
    };
    let photo = PhotoSettings {
        exposure: 1.,
        contrast: 1.3,
        ..PhotoSettings::default()
    };
    let pipeline = Pipeline::new(
        calibration,
        RollSettings::new_grade(),
        photo,
        DMat3::IDENTITY,
    )
    .unwrap();
    let empty = LocalGrade::new(vec![]).unwrap();
    let exposure = LocalGrade::new(vec![adjustment(PhotoSettings {
        exposure: 1.,
        ..PhotoSettings::default()
    })])
    .unwrap();
    let point = SourcePoint([18., 20.]);
    for scan in [DVec3::new(0., 0.05, 0.03), DVec3::new(0.3, 0.1, 0.05)] {
        let global = pipeline.apply(scan).unwrap();
        assert_eq!(empty.apply(global, point).unwrap(), global);
        let local = exposure.apply(global, point).unwrap();
        near_rgb(local.rgb, global.rgb * 2.);
        assert_eq!(local.floored, global.floored);
    }
    assert_eq!(
        photo.before_white_balance(),
        PhotoSettings {
            exposure: 1.,
            ..PhotoSettings::default()
        }
    );
}

#[test]
fn mask_sampling_uses_full_source_pixel_centres_for_proxies_and_tiles() {
    let region = SourceRegion {
        x: 101,
        y: 203,
        width: 40,
        height: 20,
    };
    let mask = GradientMask::Linear {
        start: SourcePoint([101.5, 203.5]),
        end: SourcePoint([141.5, 203.5]),
    };
    let tile = SourceRaster::new(
        SourceRegion {
            x: 109,
            y: 206,
            width: 8,
            height: 5,
        },
        [8, 5],
    )
    .unwrap();
    let proxy = SourceRaster::new(region, [7, 3]).unwrap();
    for (raster, size) in [(proxy, [7, 3]), (tile, [8, 5])] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let sample = raster.source_pixel([x, y]).unwrap();
                let point = SourcePoint(sample.map(|v| f64::from(v) + 0.5));
                let t = (f64::from(sample[0]) - 101.) / 40.;
                near(mask.coverage(point).unwrap(), t * t * (3. - 2. * t));
            }
        }
    }
}

#[test]
fn composition_coordinate_roundtrips_preserve_source_mask_coverage() {
    let area = SourceRegion {
        x: 100,
        y: 200,
        width: 80,
        height: 60,
    };
    let masks = [
        GradientMask::Linear {
            start: SourcePoint([90., 220.]),
            end: SourcePoint([170., 260.]),
        },
        GradientMask::Radial {
            center: SourcePoint([140., 230.]),
            radii: [30., 15.],
            angle: 0.3,
            feather: 0.8,
        },
    ];
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        for horizontal in [false, true] {
            for angle in [0., 0.17] {
                let composition = Composition::with_crop(
                    area,
                    RollOrientation {
                        turns,
                        horizontal,
                        vertical: false,
                    },
                    PhotoOrientation {
                        turns: QuarterTurns::One,
                        flips: Flips {
                            horizontal: false,
                            vertical: true,
                        },
                    },
                    Some(Crop {
                        center: [140., 230.],
                        size: [40, 20],
                        angle,
                    }),
                )
                .unwrap();
                for source in [SourcePoint([130.5, 225.5]), SourcePoint([145.5, 235.5])] {
                    let frame = composition.source_to_frame(source);
                    let roundtrip = composition.frame_to_source(frame);
                    for mask in masks {
                        near(
                            mask.coverage(roundtrip).unwrap(),
                            mask.coverage(source).unwrap(),
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_geometry_settings_and_strength_are_rejected_even_when_disabled() {
    for mask in [
        GradientMask::Linear {
            start: SourcePoint([1., 2.]),
            end: SourcePoint([1., 2.]),
        },
        GradientMask::Linear {
            start: SourcePoint([f64::NAN, 0.]),
            end: SourcePoint([1., 2.]),
        },
        GradientMask::Linear {
            start: SourcePoint([-f64::MAX, 0.]),
            end: SourcePoint([f64::MAX, 0.]),
        },
        GradientMask::Radial {
            center: SourcePoint([0., f64::INFINITY]),
            radii: [1., 1.],
            angle: 0.,
            feather: 0.5,
        },
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [0., 1.],
            angle: 0.,
            feather: 0.5,
        },
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [-1., 1.],
            angle: 0.,
            feather: 0.5,
        },
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [f64::INFINITY, 1.],
            angle: 0.,
            feather: 0.5,
        },
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [1., 1.],
            angle: f64::NAN,
            feather: 0.5,
        },
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [1., 1.],
            angle: 0.,
            feather: -0.1,
        },
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [1., 1.],
            angle: 0.,
            feather: 1.1,
        },
    ] {
        assert!(mask.validate().is_err());
        assert!(
            LocalGrade::new(vec![LocalAdjustment {
                mask,
                enabled: false,
                ..adjustment(PhotoSettings::default())
            }])
            .is_err()
        );
    }
    for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(weighted_settings(PhotoSettings::default(), value).is_err());
        assert!(
            LocalGrade::new(vec![LocalAdjustment {
                strength: value,
                enabled: false,
                ..adjustment(PhotoSettings::default())
            }])
            .is_err()
        );
    }
    let invalid = PhotoSettings {
        contrast: 2.,
        ..PhotoSettings::default()
    };
    assert!(weighted_settings(invalid, 0.).is_err());
    assert!(
        LocalGrade::new(vec![LocalAdjustment {
            enabled: false,
            ..adjustment(invalid)
        }])
        .is_err()
    );
}

#[test]
fn nonfinite_pixels_coordinates_and_arithmetic_overflow_reject_without_repair() {
    let empty = LocalGrade::new(vec![]).unwrap();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            empty
                .apply(pixel(DVec3::splat(value)), SourcePoint([10., 20.]))
                .is_err()
        );
        assert!(
            empty
                .apply(pixel(DVec3::ONE), SourcePoint([value, 20.]))
                .is_err()
        );
        assert!(linear().coverage(SourcePoint([10., value])).is_err());
    }
    let grade = LocalGrade::new(vec![adjustment(PhotoSettings {
        exposure: 1.,
        ..PhotoSettings::default()
    })])
    .unwrap();
    assert_eq!(
        grade.apply(pixel(DVec3::splat(f64::MAX)), SourcePoint([18., 20.])),
        Err(Error::NumericalOverflow)
    );
    let tiny = GradientMask::Radial {
        center: SourcePoint([0., 0.]),
        radii: [f64::MIN_POSITIVE, 1.],
        angle: 0.,
        feather: 1.,
    };
    assert_eq!(
        tiny.coverage(SourcePoint([f64::MAX, 0.])),
        Err(Error::NumericalOverflow)
    );
}
