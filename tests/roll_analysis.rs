use antitone::{
    Error,
    color::{AP1_TO_XYZ, Balance, wb_matrix},
    processing::{
        Calibration, DEFAULT_GAMMA, Inversion, NegativeInput, PhotoSettings, Pipeline, Response,
    },
    roll_analysis::{FrameSamples, MAX_SAMPLES_PER_FRAME, estimate},
};
use glam::{DMat3, DVec3};

const FLOOR: f64 = 1. / 65535.;
const BASE: DVec3 = DVec3::new(0.8, 0.4, 0.2);

fn picture(n: usize, lo: f64, hi: f64) -> Vec<DVec3> {
    (0..n)
        .map(|i| BASE * (lo + (hi - lo) * i as f64 / (n - 1) as f64))
        .collect()
}

fn current(gamma: f64, response: Response) -> NegativeInput {
    NegativeInput {
        calibration: Calibration {
            dmin: BASE,
            dmax: BASE * 0.02,
            input_floor: FLOOR,
        },
        response,
        gamma,
        balance: Balance::default(),
    }
}

fn default_current() -> NegativeInput {
    current(DEFAULT_GAMMA, Response::film())
}

fn proposal(
    frames: &[FrameSamples<'_>],
    matrix: DMat3,
    current: NegativeInput,
) -> antitone::roll_analysis::Proposal {
    estimate(frames, FLOOR, matrix, current, || false)
        .unwrap()
        .unwrap()
}

fn negative(p: &antitone::roll_analysis::Proposal) -> NegativeInput {
    p.input.negative_input().unwrap()
}

fn calibration(p: &antitone::roll_analysis::Proposal) -> Calibration {
    p.input.calibration().unwrap()
}

fn close(a: DVec3, b: DVec3) {
    assert!((a - b).abs().max_element() < 1e-11, "{a:?} != {b:?}");
}

#[test]
fn per_channel_development_scale_preserves_the_analyzed_positive_look() {
    let scale = DVec3::new(2., 1.25, 1.6);
    let film_unity = vec![BASE; 256];
    let film_scaled: Vec<_> = film_unity.iter().map(|&v| v * scale).collect();
    let source_unity = picture(1001, 0.05, 0.8);
    let source_scaled: Vec<_> = source_unity.iter().map(|&v| v * scale).collect();
    let matrix = DMat3::from_diagonal(DVec3::new(1.3, 0.8, 1.1));
    let unity = proposal(
        &[FrameSamples {
            film: &film_unity,
            picture: &source_unity,
        }],
        matrix,
        default_current(),
    );
    let scaled_current = NegativeInput {
        calibration: Calibration {
            dmin: BASE * scale,
            dmax: BASE * 0.02 * scale,
            input_floor: FLOOR,
        },
        ..default_current()
    };
    let scaled = proposal(
        &[FrameSamples {
            film: &film_scaled,
            picture: &source_scaled,
        }],
        matrix,
        scaled_current,
    );

    close(calibration(&scaled).dmin, calibration(&unity).dmin * scale);
    close(calibration(&scaled).dmax, calibration(&unity).dmax * scale);
    assert!((negative(&scaled).balance.warmth - negative(&unity).balance.warmth).abs() < 1e-9);
    assert!((negative(&scaled).balance.tint - negative(&unity).balance.tint).abs() < 1e-9);
    assert!((scaled.roll.wb.warmth - unity.roll.wb.warmth).abs() < 1e-9);
    assert!((scaled.roll.wb.tint - unity.roll.wb.tint).abs() < 1e-9);
    assert!((scaled.roll.exposure - unity.roll.exposure).abs() < 1e-9);
    assert!((scaled.photos[0].exposure - unity.photos[0].exposure).abs() < 1e-9);

    let unity_pipeline = Pipeline::new(unity.input, unity.roll, unity.photos[0], matrix).unwrap();
    let scaled_pipeline =
        Pipeline::new(scaled.input, scaled.roll, scaled.photos[0], matrix).unwrap();
    for (&a, &b) in source_unity.iter().zip(&source_scaled) {
        let a = unity_pipeline.apply(a).unwrap().rgb;
        let b = scaled_pipeline.apply(b).unwrap().rgb;
        assert!((a - b).abs().max_element() < 1e-10, "{a:?} != {b:?}");
    }
}

#[test]
fn usable_film_is_pooled_without_requiring_every_frame_to_supply_base() {
    let film = vec![BASE; 256];
    let a = picture(512, 0.05, 0.5);
    let b = picture(512, 0.1, 0.8);
    let current = current(0.9, Response::film());
    let p = proposal(
        &[
            FrameSamples {
                film: &film,
                picture: &a,
            },
            FrameSamples {
                film: &[],
                picture: &b,
            },
        ],
        DMat3::IDENTITY,
        current,
    );
    close(calibration(&p).dmin, BASE);
    assert_eq!(negative(&p).gamma, current.gamma);
    assert_eq!(negative(&p).response, current.response);
    assert_eq!(p.frames[1].film_samples, 0);
    assert!(
        p.frames
            .iter()
            .all(|s| s.balance_samples == 512 && s.exposure_samples == 512)
    );
    assert!(negative(&p).balance.warmth.abs() < 1e-9 && negative(&p).balance.tint.abs() < 1e-9);
    assert_ne!(p.roll.exposure, 0.);
    let inv = Inversion::new(negative(&p)).unwrap();
    let mut totals = Vec::new();
    for (index, pixels) in [&a, &b].into_iter().enumerate() {
        let mut y: Vec<_> = pixels
            .iter()
            .map(|&v| (AP1_TO_XYZ * (wb_matrix(p.roll.wb).unwrap() * inv.apply(v).unwrap().rgb)).y)
            .collect();
        y.sort_by(f64::total_cmp);
        let at = (y.len() - 1) as f64 * 0.99;
        let p99 = y[at.floor() as usize] * (1. - at.fract()) + y[at.ceil() as usize] * at.fract();
        let mean = y.iter().map(|v| v.min(p99)).sum::<f64>() / y.len() as f64;
        let total = ((0.18 / mean).log2() - p99.log2()) * 0.5;
        totals.push(total);
        assert!((p.roll.exposure + p.photos[index].exposure - total).abs() < 1e-11);
        assert_eq!(
            p.photos[index],
            PhotoSettings {
                exposure: p.photos[index].exposure,
                ..PhotoSettings::default()
            }
        );
    }
    assert!((p.roll.exposure - (totals[0] + totals[1]) * 0.5).abs() < 1e-11);
    assert!((p.photos[0].exposure + p.photos[1].exposure).abs() < 1e-11);
}

#[test]
fn dmax_channels_select_separate_lower_quantiles_not_one_joint_pixel() {
    let film = vec![BASE; 256];
    let n = 1001;
    let source: Vec<_> = (0..n)
        .map(|i| {
            let t = |j: usize| 0.05 + 0.75 * j as f64 / 1000.;
            BASE * DVec3::new(t(i), t(1000 - i), t((i + 500) % n))
        })
        .collect();
    let p = proposal(
        &[FrameSamples {
            film: &film,
            picture: &source,
        }],
        DMat3::IDENTITY,
        default_current(),
    );
    close(calibration(&p).dmax, BASE * 0.05375);
    assert!(!source.iter().any(|v| (*v / BASE).max_element() < 0.08));
}

#[test]
fn repeated_samples_do_not_increase_a_frames_endpoint_or_curve_influence() {
    let film = vec![BASE; 256];
    let a = picture(512, 0.05, 0.3);
    let b: Vec<_> = picture(512, 0.3, 0.85)
        .into_iter()
        .map(|v| {
            let t = v / BASE;
            BASE * DVec3::new(t.x.powf(1.05), t.y.powf(0.97), t.z)
        })
        .collect();
    let repeated: Vec<_> = b.iter().flat_map(|v| std::iter::repeat_n(*v, 4)).collect();
    let matrix = DMat3::from_diagonal(DVec3::new(1.3, 0.8, 1.1));
    let fit = |second: &[DVec3]| {
        proposal(
            &[
                FrameSamples {
                    film: &film,
                    picture: &a,
                },
                FrameSamples {
                    film: &[],
                    picture: second,
                },
            ],
            matrix,
            default_current(),
        )
    };
    let original = fit(&b);
    let duplicated = fit(&repeated);
    close(calibration(&original).dmax, calibration(&duplicated).dmax);
    assert!(
        (negative(&original).balance.warmth - negative(&duplicated).balance.warmth).abs() < 1e-11
    );
    assert!((negative(&original).balance.tint - negative(&duplicated).balance.tint).abs() < 1e-11);
    assert!((original.roll.wb.warmth - duplicated.roll.wb.warmth).abs() < 1e-11);
    assert!((original.roll.wb.tint - duplicated.roll.wb.tint).abs() < 1e-11);
    assert!(duplicated.frames.iter().all(|s| s.balance_samples == 512));
}

#[test]
fn shared_wb_neutralizes_the_roll_mean_without_fitting_channel_curves_to_gray() {
    let film = vec![BASE; 256];
    let source = picture(1001, 0.05, 0.8);
    let matrix = DMat3::from_diagonal(DVec3::new(1.3, 0.8, 1.1));
    let p = proposal(
        &[FrameSamples {
            film: &film,
            picture: &source,
        }],
        matrix,
        default_current(),
    );
    let inv = Inversion::new(negative(&p)).unwrap();
    let positives: Vec<_> = source
        .iter()
        .map(|&v| wb_matrix(p.roll.wb).unwrap() * matrix * inv.apply(v).unwrap().rgb)
        .collect();
    let m = antitone::sampling::mean(&positives).unwrap();
    assert!((m / m.y - DVec3::ONE).abs().max_element() < 1e-8);
    assert!(negative(&p).balance.warmth.abs() + negative(&p).balance.tint.abs() < 1e-6);
    assert!(p.roll.wb.warmth.abs() + p.roll.wb.tint.abs() > 0.01);
}

#[test]
fn out_of_range_shared_wb_is_rejected_not_clamped_or_replaced_by_identity() {
    let film = vec![BASE; 256];
    let source = picture(512, 0.05, 0.8);
    let cast = wb_matrix(Balance {
        warmth: 0.8,
        tint: 0.,
    })
    .unwrap()
    .inverse();
    assert!(matches!(
        estimate(
            &[FrameSamples {
                film: &film,
                picture: &source
            }],
            FLOOR,
            cast * cast,
            default_current(),
            || false
        ),
        Err(Error::UnusableSample(
            "WB correction is outside the control range"
        ))
    ));
}

#[test]
fn one_hdr_outlier_does_not_move_the_p99_capped_exposure_prior_or_clamp_working_pixels() {
    let film = vec![BASE; 256];
    let mut source = vec![BASE * 0.3; 8192];
    let fit = |pixels: &[DVec3]| {
        proposal(
            &[FrameSamples {
                film: &film,
                picture: pixels,
            }],
            DMat3::IDENTITY,
            default_current(),
        )
    };
    let before = fit(&source);
    // One sample cannot move either endpoint quantile; every channel stays above the floor.
    source[0] = BASE * 0.0001;
    let after = fit(&source);
    close(calibration(&before).dmin, calibration(&after).dmin);
    close(calibration(&before).dmax, calibration(&after).dmax);
    assert!((after.roll.exposure - before.roll.exposure).abs() < 1e-14);
    assert!((after.photos[0].exposure - before.photos[0].exposure).abs() < 1e-14);
    let pipeline =
        Pipeline::new(after.input, after.roll, after.photos[0], DMat3::IDENTITY).unwrap();
    let y: Vec<_> = source
        .iter()
        .map(|&p| (AP1_TO_XYZ * pipeline.apply(p).unwrap().rgb).y)
        .collect();
    assert!(
        y[0] > 65536.,
        "y0={} y1={} cal={:?}",
        y[0],
        y[1],
        calibration(&after)
    );
    let uncapped_mean = y.iter().sum::<f64>() / y.len() as f64;
    assert!((uncapped_mean / y[1]).log2() * 0.5 > 2.);
    assert_eq!(after.frames[0].exposure_samples, source.len());
}

#[test]
fn invalid_inputs_and_unusable_frames_do_not_produce_partial_proposals() {
    let film = vec![BASE; 256];
    let source = picture(512, 0.05, 0.8);
    let frame = FrameSamples {
        film: &film,
        picture: &source,
    };
    assert!(estimate(&[], FLOOR, DMat3::IDENTITY, default_current(), || false).is_err());
    assert!(
        estimate(
            &[FrameSamples {
                picture: &[],
                ..frame
            }],
            FLOOR,
            DMat3::IDENTITY,
            default_current(),
            || false
        )
        .is_err()
    );
    assert!(
        estimate(
            &[FrameSamples { film: &[], ..frame }],
            FLOOR,
            DMat3::IDENTITY,
            default_current(),
            || false
        )
        .is_err()
    );
    assert!(estimate(&[frame], 0., DMat3::IDENTITY, default_current(), || false).is_err());
    assert!(estimate(&[frame], FLOOR, DMat3::ZERO, default_current(), || false).is_err());
    let too_many = vec![BASE; MAX_SAMPLES_PER_FRAME + 1];
    assert!(matches!(
        estimate(
            &[FrameSamples {
                film: &too_many,
                ..frame
            }],
            FLOOR,
            DMat3::IDENTITY,
            default_current(),
            || false
        ),
        Err(Error::InvalidParameter(_))
    ));
    for value in [f64::NAN, FLOOR, 0.] {
        let bad = [DVec3::new(value, 0.1, 0.1)];
        assert!(
            estimate(
                &[FrameSamples {
                    picture: &bad,
                    ..frame
                }],
                FLOOR,
                DMat3::IDENTITY,
                default_current(),
                || false
            )
            .is_err()
        );
    }
    let at_base = vec![BASE; 512];
    assert!(
        estimate(
            &[
                frame,
                FrameSamples {
                    film: &[],
                    picture: &at_base
                }
            ],
            FLOOR,
            DMat3::IDENTITY,
            default_current(),
            || false
        )
        .is_err()
    );
}

#[test]
fn cooperative_cancellation_at_start_during_fitting_and_before_completion_changes_nothing() {
    let film = vec![BASE; 256];
    let source = picture(512, 0.05, 0.8);
    let original = source.clone();
    let frames = [FrameSamples {
        film: &film,
        picture: &source,
    }];
    let current = NegativeInput {
        calibration: Calibration {
            dmin: BASE,
            dmax: BASE * 0.02,
            input_floor: FLOOR,
        },
        response: Response::film(),
        gamma: DEFAULT_GAMMA,
        balance: Balance {
            warmth: 0.1,
            tint: -0.05,
        },
    };
    let mut checkpoints = 0;
    assert!(
        estimate(&frames, FLOOR, DMat3::IDENTITY, current, || {
            checkpoints += 1;
            false
        })
        .unwrap()
        .is_some()
    );
    for stop in [
        1,
        2,
        3,
        checkpoints / 8,
        checkpoints / 2,
        checkpoints - 1,
        checkpoints,
    ] {
        let mut calls = 0;
        let result = estimate(&frames, FLOOR, DMat3::IDENTITY, current, || {
            calls += 1;
            calls >= stop
        })
        .unwrap();
        assert!(result.is_none(), "checkpoint {stop}");
    }
    assert_eq!(source, original);
    assert_eq!(film, vec![BASE; 256]);
}
