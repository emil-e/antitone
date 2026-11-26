use super::*;
use crate::color::opponent;

struct AuthoredPositives;

impl ForwardModel for AuthoredPositives {
    fn read(
        &mut self,
        _: NegativeInput,
        points: &[DVec3],
        cancelled: &mut dyn FnMut() -> bool,
        consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        for (i, &point) in points.iter().enumerate() {
            if cancelled() {
                return Ok(false);
            }
            consume(i, point)?;
        }
        Ok(true)
    }
}

fn estimate_authored(
    groups: &[&[DVec3]],
    matrix: DMat3,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<(Balance, Vec<usize>)>, Error> {
    estimate(
        &SampleBatch::new(groups.iter().copied()),
        NegativeInput {
            calibration: Calibration {
                dmin: DVec3::splat(0.5),
                dmax: DVec3::splat(0.03),
            },
            response: crate::processing::Response {
                toe: 1.,
                shoulder: 0.,
            },
            gamma: 0.6,
            balance: Balance::default(),
        },
        &vec![matrix; groups.len()],
        &mut AuthoredPositives,
        &mut cancelled,
    )
}

fn cast(warmth: f64, tint: f64, level: f64) -> DVec3 {
    let to_cones = ap1_to_cones();
    to_cones.inverse() * ((to_cones * DVec3::ONE) * opponent(warmth, tint).exp() * level)
}

#[test]
fn averages_log_chroma_within_each_photo_then_uses_the_median_across_photos() {
    // Widely different brightnesses must not affect the log-colour mean.
    let matrix = DMat3::from_diagonal(DVec3::new(0.7, 1.1, 1.4));
    let from_ap1 = matrix.inverse();
    let first = [
        from_ap1 * cast(-0.6, 0.8, 1e-12),
        from_ap1 * cast(0.2, 0., 30.),
    ];
    let middle = [from_ap1 * cast(0.1, -0.2, 0.18)];
    let abundant = vec![from_ap1 * cast(0.9, 0.5, 0.3); 1600];
    let invalid = [DVec3::ZERO; 3];
    let (wb, counts) = estimate_authored(&[&first, &middle, &abundant, &invalid], matrix, || false)
        .unwrap()
        .unwrap();
    assert_eq!(counts, [2, 1, 1024, 0]);
    assert!((wb.warmth + 0.1).abs() < 1e-12);
    assert!((wb.tint + 0.4).abs() < 1e-12);
}

#[test]
fn a_strong_vote_is_not_clamped_until_after_pooling() {
    let a = [cast(1.6, -0.3, 0.2)];
    let b = [cast(0., -0.1, 0.2)];
    let (wb, _) = estimate_authored(&[&a, &b], DMat3::IDENTITY, || false)
        .unwrap()
        .unwrap();
    assert!((wb.warmth + 0.8).abs() < 1e-12);
    assert!((wb.tint - 0.2).abs() < 1e-12);
    let (wb, _) = estimate_authored(&[&a], DMat3::IDENTITY, || false)
        .unwrap()
        .unwrap();
    assert_eq!(wb.warmth, -1.);
}

#[cfg(feature = "editor")]
#[test]
fn semantic_refit_only_grades_the_roll_and_meters_abstaining_photos() {
    let first = vec![cast(-0.4, 0.2, 1e-12); 300];
    let second = vec![cast(0.2, -0.2, 0.18); 300];
    let withheld = vec![cast(0.9, 0.7, 0.18); 300];
    let matrices = [
        DMat3::from_diagonal(DVec3::new(0.7, 1.1, 1.4)),
        DMat3::from_diagonal(DVec3::new(1.8, 0.6, 0.9)),
        DMat3::from_diagonal(DVec3::new(1.2, 1.5, 0.8)),
    ];
    let camera: Vec<Vec<_>> = [&first, &second, &withheld]
        .into_iter()
        .zip(matrices)
        .map(|(picture, matrix)| picture.iter().map(|p| matrix.inverse() * *p).collect())
        .collect();
    let frames: Vec<_> = camera
        .iter()
        .zip(matrices)
        .map(|(picture, matrix)| FrameSamples {
            camera_to_ap1: matrix,
            film: &[],
            picture,
        })
        .collect();
    let input = InputSettings::negative(Calibration {
        dmin: DVec3::splat(0.5),
        dmax: DVec3::splat(0.03),
    });
    let mut proposal = Proposal {
        input,
        roll: RollSettings::default(),
        photos: vec![AdjustmentLayerSettings::default(); 3],
        frames: vec![
            FrameStatistics {
                film_samples: 0,
                picture_samples: 300,
                balance_eligible: 300,
                balance_samples: 300,
                exposure_samples: 0,
                grayness: None
            };
            3
        ],
    };
    assert!(
        semantic_refit(
            &mut proposal,
            &frames,
            &mut AuthoredPositives,
            &mut || false,
            |frame, indices, _| {
                assert_eq!(indices.len(), 300);
                Ok(Some(vec![frame != 2; indices.len()]))
            }
        )
        .unwrap()
    );
    assert!((proposal.roll.wb.warmth - 0.1).abs() < 1e-12);
    assert!(
        proposal
            .photos
            .iter()
            .all(|p| p.wb == Balance::default() && p.exposure == 0.)
    );
    assert_eq!(proposal.frames[0].exposure_samples, 300);
    assert_eq!(proposal.frames[2].balance_samples, 0);
    assert_eq!(proposal.frames[2].exposure_samples, 300);
}

#[test]
fn no_usable_picture_chroma_is_a_failure_but_cancellation_is_not() {
    let invalid = [DVec3::ZERO];
    assert!(matches!(
        estimate_authored(&[&invalid], DMat3::IDENTITY, || false),
        Err(Error::UnusableSample(_))
    ));
    assert!(
        estimate_authored(&[&invalid], DMat3::IDENTITY, || true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn mixed_camera_mappings_supply_each_photos_colour_vote_and_exposure() {
    let matrices = [
        DMat3::from_diagonal(DVec3::new(0.7, 1.1, 1.4)),
        DMat3::from_diagonal(DVec3::new(1.8, 0.6, 0.9)),
        DMat3::from_diagonal(DVec3::new(1.2, 1.5, 0.8)),
    ];
    let positives = [
        cast(-0.4, 0.2, 0.09),
        cast(0.1, -0.2, 0.18),
        cast(0.6, 0.5, 0.72),
    ];
    let camera = std::array::from_fn::<_, 3, _>(|i| [matrices[i].inverse() * positives[i]]);
    let picture = SampleBatch::new(camera.iter().map(|p| p.as_slice()));
    let negative = InputSettings::negative(Calibration {
        dmin: DVec3::splat(0.5),
        dmax: DVec3::splat(0.03),
    })
    .negative_input()
    .unwrap();
    let (wb, counts) = estimate(
        &picture,
        negative,
        &matrices,
        &mut AuthoredPositives,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(counts, [1, 1, 1]);
    assert!((wb.warmth + 0.1).abs() < 1e-12);
    assert!((wb.tint + 0.2).abs() < 1e-12);
    let (exposure, counts) = meter(
        &picture,
        negative,
        &matrices,
        wb,
        &mut AuthoredPositives,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(counts, [1, 1, 1]);
    let mut luminances = positives.map(|p| (AP1_TO_XYZ * wb_matrix(wb).unwrap() * p).y);
    luminances.sort_by(f64::total_cmp);
    assert!((exposure - (0.18 / luminances[1]).log2()).abs() < 1e-12);
}
