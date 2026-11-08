use super::*;
use crate::processing::{DEFAULT_GAMMA, Response};

type AuthoredRoll = (
    NegativeInput,
    Vec<SpatialSamples>,
    Vec<Vec<DVec3>>,
    Vec<Vec<DVec3>>,
);

fn authored() -> AuthoredRoll {
    authored_directions(
        &[
            Balance {
                warmth: 0.2,
                tint: -0.1,
            },
            Balance {
                warmth: 0.7,
                tint: 0.3,
            },
            Balance {
                warmth: 0.3,
                tint: 0.05,
            },
        ]
        .map(|cast| wb_matrix(cast).unwrap() * DVec3::ONE),
    )
}

fn authored_directions(directions: &[DVec3]) -> AuthoredRoll {
    let negative = NegativeInput {
        calibration: Calibration {
            dmin: DVec3::splat(0.5),
            dmax: DVec3::splat(0.03),
            input_floor: 1. / 65535.,
        },
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
        gamma: DEFAULT_GAMMA,
        balance: Balance {
            warmth: -0.25,
            tint: 0.07,
        },
    };
    let mut images = Vec::new();
    let mut pictures = Vec::new();
    let mut positives = Vec::new();
    for &direction in directions {
        let size = [128, 96];
        let shades: Vec<_> = (0..size[0] * size[1])
            .map(|i| {
                let x = (i % size[0]) as f64;
                let y = (i / size[0]) as f64;
                direction * (0.04 + 0.6 * ((x * 0.09).sin() * (y * 0.11).cos() + 1.) / 2.)
            })
            .collect();
        // Analytic inverse of the toe=1, shoulder=0 response, not an image
        // recorded from the estimator. All three channels share known scene light.
        let gammas = negative.channel_gammas().unwrap();
        let span = (negative.calibration.dmin / negative.calibration.dmax).ln();
        let raw: Vec<_> = shades
            .iter()
            .map(|p| {
                DVec3::from_array(std::array::from_fn(|c| {
                    negative.calibration.dmin[c]
                        * (1. + p[c] * (span[c] / gammas[c]).exp_m1()).powf(-gammas[c])
                }))
            })
            .collect();
        let indices: Vec<_> = (0..512).map(|i| (2 * i + 1) * raw.len() / 1024).collect();
        pictures.push(indices.iter().map(|&i| raw[i]).collect());
        positives.push(indices.iter().map(|&i| shades[i]).collect());
        images.push(SpatialSamples::new(size, raw, vec![true; shades.len()]).unwrap());
    }
    (negative, images, pictures, positives)
}

fn seed(negative: NegativeInput, count: usize) -> Proposal {
    Proposal {
        input: NegativeInput {
            balance: Balance::default(),
            ..negative
        }
        .into(),
        roll: RollSettings {
            exposure: 0.,
            wb: Balance {
                warmth: 0.6,
                tint: -0.3,
            },
        },
        photos: vec![AdjustmentLayerSettings::default(); count],
        frames: vec![
            FrameStatistics {
                film_samples: 512,
                picture_samples: 512,
                balance_eligible: 512,
                balance_samples: 512,
                exposure_samples: 512,
                grayness: None
            };
            count
        ],
    }
}

#[test]
fn one_pass_recovers_authored_shared_curves_and_refreshes_only_shared_wb_and_exposure() {
    let (truth, images, pictures, positives) = authored();
    let frames: Vec<_> = pictures
        .iter()
        .map(|p| FrameSamples {
            film: &[],
            picture: p,
        })
        .collect();
    let refs: Vec<_> = images.iter().collect();
    let mut model = Observed {
        raster_reads: Vec::new(),
        fail_final: false,
    };
    let result = refine(
        &frames,
        &refs,
        DMat3::IDENTITY,
        truth.calibration.input_floor,
        seed(truth, 3),
        &mut model,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    let balance = result.input.negative_input().unwrap().balance;
    assert_eq!(
        model.raster_reads.len(),
        6,
        "one initial GI selection and one final WB selection per photo"
    );
    assert!(
        model.raster_reads[..3]
            .iter()
            .all(|g| *g == Balance::default())
    );
    assert!(model.raster_reads[3..].iter().all(|g| *g == balance));
    assert!(
        (balance.warmth - truth.balance.warmth)
            .abs()
            .max((balance.tint - truth.balance.tint).abs())
            < 2e-5
    );
    assert!(
        (result.roll.wb.warmth + 0.3)
            .abs()
            .max((result.roll.wb.tint + 0.05).abs())
            < 2e-5
    );
    assert!(
        result
            .photos
            .iter()
            .all(|p| *p == AdjustmentLayerSettings::default())
    );
    assert!(
        result
            .frames
            .iter()
            .all(|f| f.grayness.as_ref().unwrap().wb_rejection.is_none())
    );
    let mut medians: Vec<_> = positives
        .iter()
        .map(|frame| {
            let mut y: Vec<_> = frame
                .iter()
                .map(|p| {
                    (AP1_TO_XYZ
                        * (wb_matrix(Balance {
                            warmth: -0.3,
                            tint: -0.05,
                        })
                        .unwrap()
                            * *p))
                        .y
                })
                .collect();
            quantile(&mut y, 0.5)
        })
        .collect();
    let expected_ev = (0.18 / quantile(&mut medians, 0.5)).log2();
    assert!((result.roll.exposure - expected_ev).abs() < 1e-4);
    let pair = refine(
        &frames[..2],
        &refs[..2],
        DMat3::IDENTITY,
        truth.calibration.input_floor,
        seed(truth, 2),
        &mut model::ScalarReference,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert!(
        (pair.roll.wb.warmth + 0.45)
            .abs()
            .max((pair.roll.wb.tint + 0.1).abs())
            < 2e-5
    );
}

#[test]
fn invalid_layout_and_unusable_frame_do_not_become_successful_or_silently_dropped() {
    assert!(SpatialSamples::new([2, 2], vec![DVec3::ONE; 3], vec![true; 4]).is_err());
    assert!(SpatialSamples::new([2, 2], vec![DVec3::ONE; 4], vec![true; 3]).is_err());
    assert!(SpatialSamples::new([0, 2], Vec::new(), Vec::new()).is_err());
    assert!(matches!(
        SpatialSamples::new([1, 1], vec![DVec3::NAN], vec![true]),
        Err(Error::NumericalOverflow)
    ));
    assert!(SpatialSamples::new([1, 1], vec![DVec3::NAN], vec![false]).is_ok());
    assert!(SpatialSamples::new([513, 1], vec![DVec3::ONE; 513], vec![true; 513]).is_err());
    let (truth, mut images, pictures, _) = authored();
    images[1].valid.fill(false);
    let frames: Vec<_> = pictures
        .iter()
        .map(|p| FrameSamples {
            film: &[],
            picture: p,
        })
        .collect();
    let refs: Vec<_> = images.iter().collect();
    let error = refine(
        &frames,
        &refs,
        DMat3::IDENTITY,
        truth.calibration.input_floor,
        seed(truth, 3),
        &mut model::ScalarReference,
        &mut || false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("photo 2 GI gamma evidence"));
    assert!(
        refine(
            &frames,
            &refs,
            DMat3::IDENTITY,
            truth.calibration.input_floor,
            seed(truth, 3),
            &mut model::ScalarReference,
            &mut || true
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn forward_failures_and_incomplete_reads_propagate_instead_of_becoming_wb_abstentions() {
    struct Broken(bool);
    impl ForwardModel for Broken {
        fn read(
            &mut self,
            _: NegativeInput,
            _: &[DVec3],
            _: &mut dyn FnMut() -> bool,
            _: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
        ) -> Result<bool, Error> {
            if self.0 {
                Err(Error::UnusableSample("authored forward failure"))
            } else {
                Ok(true)
            }
        }
    }
    let (truth, images, _, _) = authored();
    assert!(matches!(
        images[0].response(truth, &mut Broken(true), &mut || false),
        Err(Error::UnusableSample("authored forward failure"))
    ));
    assert!(matches!(
        images[0].response(truth, &mut Broken(false), &mut || false),
        Err(Error::InvalidParameter("analysis model omitted samples"))
    ));
}

#[test]
fn spatial_estimation_keeps_the_effective_response_floor_separate_from_the_saved_native_floor() {
    struct FloorGuard {
        common: f64,
        calls: usize,
        spatial_reads: usize,
    }
    impl ForwardModel for FloorGuard {
        fn read(
            &mut self,
            negative: NegativeInput,
            points: &[DVec3],
            cancelled: &mut dyn FnMut() -> bool,
            consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
        ) -> Result<bool, Error> {
            assert_eq!(negative.calibration.input_floor, self.common);
            assert!(
                points.iter().all(|p| p.min_element() > self.common),
                "locally usable but below-common-floor sites must not reach the response model"
            );
            self.calls += 1;
            self.spatial_reads += usize::from(points.len() >= 128 * 96 - 1);
            model::ScalarReference.read(negative, points, cancelled, consume)
        }
    }
    let (truth, mut images, pictures, _) = authored();
    let common = 0.005;
    let locally_usable = DVec3::splat(common / 2.);
    assert!(locally_usable.min_element() > truth.calibration.input_floor);
    images[0].camera[48 * 128 + 64] = locally_usable;
    assert!(images[0].valid[48 * 128 + 64]);
    let film = vec![truth.calibration.dmin; 512];
    let frames: Vec<_> = pictures
        .iter()
        .zip(&images)
        .map(|(picture, spatial)| SpatialFrameSamples {
            statistics: FrameSamples {
                film: &film,
                picture,
            },
            spatial,
        })
        .collect();
    let mut guard = FloorGuard {
        common,
        calls: 0,
        spatial_reads: 0,
    };
    let proposal = estimate_spatial_with_model(
        &frames,
        common,
        DMat3::IDENTITY,
        truth,
        &mut guard,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert!(guard.calls > 0 && guard.spatial_reads > 0);
    assert_eq!(
        proposal
            .input
            .negative_input()
            .unwrap()
            .calibration
            .input_floor,
        truth.calibration.input_floor
    );
}

struct Observed {
    raster_reads: Vec<Balance>,
    fail_final: bool,
}
impl ForwardModel for Observed {
    fn read(
        &mut self,
        negative: NegativeInput,
        points: &[DVec3],
        cancelled: &mut dyn FnMut() -> bool,
        consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        if points.len() == 128 * 96 {
            self.raster_reads.push(negative.balance);
            if self.fail_final && self.raster_reads.len() > 3 {
                return Err(Error::UnusableSample("authored final response failure"));
            }
        }
        model::ScalarReference.read(negative, points, cancelled, consume)
    }
}

#[test]
fn final_forward_errors_are_not_misreported_as_ordinary_wb_rejections() {
    let (truth, images, pictures, _) = authored();
    let frames: Vec<_> = pictures
        .iter()
        .map(|p| FrameSamples {
            film: &[],
            picture: p,
        })
        .collect();
    let refs: Vec<_> = images.iter().collect();
    let mut model = Observed {
        raster_reads: Vec::new(),
        fail_final: true,
    };
    assert!(matches!(
        refine(
            &frames,
            &refs,
            DMat3::IDENTITY,
            truth.calibration.input_floor,
            seed(truth, 3),
            &mut model,
            &mut || false
        ),
        Err(Error::UnusableSample("authored final response failure"))
    ));
    assert_eq!(model.raster_reads.len(), 4);
}

#[test]
fn wb_range_abstention_does_not_drop_a_photo_from_gamma_or_install_individual_corrections() {
    let casts = [
        Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        Balance {
            warmth: 0.7,
            tint: 0.3,
        },
        Balance {
            warmth: 0.3,
            tint: 0.05,
        },
    ];
    let mut directions: Vec<_> = casts.map(|c| wb_matrix(c).unwrap() * DVec3::ONE).into();
    let coloured = DVec3::new(3., 1., 0.2);
    assert!(matches!(
        neutral_wb(coloured),
        Err(Error::UnusableSample(
            "WB correction is outside the control range"
        ))
    ));
    directions.push(coloured);
    let (truth, images, pictures, _) = authored_directions(&directions);
    let frames: Vec<_> = pictures
        .iter()
        .map(|p| FrameSamples {
            film: &[],
            picture: p,
        })
        .collect();
    let refs: Vec<_> = images.iter().collect();
    let proposal = refine(
        &frames,
        &refs,
        DMat3::IDENTITY,
        truth.calibration.input_floor,
        seed(truth, 4),
        &mut model::ScalarReference,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(proposal.frames.len(), 4);
    assert!(
        proposal
            .frames
            .iter()
            .all(|f| f.grayness.as_ref().unwrap().gamma_samples >= 10)
    );
    assert!(
        proposal.frames[..3]
            .iter()
            .all(|f| f.grayness.as_ref().unwrap().wb_rejection.is_none())
    );
    let rejected = proposal.frames[3].grayness.as_ref().unwrap();
    assert!(rejected.wb_samples > 0);
    assert!(
        rejected
            .wb_rejection
            .as_ref()
            .unwrap()
            .contains("outside the control range")
    );
    assert!(
        (proposal.roll.wb.warmth + 0.3)
            .abs()
            .max((proposal.roll.wb.tint + 0.05).abs())
            < 2e-5
    );
    assert!(
        proposal
            .photos
            .iter()
            .all(|p| *p == AdjustmentLayerSettings::default())
    );
    let frames = &frames[3..];
    let refs = &refs[3..];
    assert!(matches!(
        refine(
            frames,
            refs,
            DMat3::IDENTITY,
            truth.calibration.input_floor,
            seed(truth, 1),
            &mut model::ScalarReference,
            &mut || false
        ),
        Err(Error::UnusableSample(
            "no admitted GI white-balance estimates"
        ))
    ));
}
