use super::*;
use crate::{
    color::Balance,
    processing::{Calibration, Inversion, Response},
};
use glam::DMat3;

fn negative() -> NegativeInput {
    NegativeInput {
        calibration: Calibration {
            dmin: DVec3::new(0.14, 0.098, 0.094),
            dmax: DVec3::new(0.017, 0.0052, 0.0029),
            input_floor: 1. / 65535.,
        },
        gamma: 0.6,
        balance: Balance::default(),
        response: Response::film(),
    }
}

#[test]
fn response_batches_keep_sample_order_signed_hdr_values_and_reuse_parameterized_programs() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = evaluation::Context::new(&device, evaluation::CachePolicy { generated_bytes: 0 });
    let gate = SubmissionGate::default();
    let mut model = ResponseModel::new(&context, (&queue, &gate));
    let negative = negative();
    let kinds = [
        negative.calibration.dmin * 1.1,
        DVec3::splat(negative.calibration.input_floor * 1.05),
        DVec3::new(0.04, 0.03, 0.05),
        DVec3::new(0.018, 0.006, 0.003),
        DVec3::splat(2.),
    ];
    let points: Vec<_> = (0..70_003)
        .map(|index| kinds[index % kinds.len()].as_vec3().as_dvec3())
        .collect();
    let scalar = Inversion::new(negative).unwrap();
    let mut count = 0;
    let mut signed = false;
    let mut hdr = false;
    let mut first = Vec::new();
    assert!(
        model
            .read(negative, &points, &mut || false, &mut |index, actual| {
                assert_eq!(index, count);
                let expected = scalar.apply(points[index]).unwrap().rgb;
                assert!(
                    (actual - expected).abs().max_element()
                        < 2e-5 * expected.abs().max_element().max(1.),
                    "{index}: {actual:?} != {expected:?}"
                );
                signed |= expected.min_element() < 0.;
                hdr |= expected.max_element() > 1.;
                if index < 3 {
                    first.push(actual);
                }
                count += 1;
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(count, points.len());
    assert!(
        signed && hdr,
        "the response fixture must exercise both continuations"
    );
    let programs = context.programs.len();
    let changed = NegativeInput {
        balance: Balance {
            warmth: 0.1,
            tint: -0.05,
        },
        ..negative
    };
    let mut difference: f64 = 0.;
    model
        .read(
            changed,
            &points[..3],
            &mut || false,
            &mut |index, actual| {
                difference = difference.max((actual - first[index]).abs().max_element());
                Ok(())
            },
        )
        .unwrap();
    assert!(
        difference > 0.01,
        "the fixture must distinguish candidate parameters"
    );
    assert_eq!(context.programs.len(), programs);
    model
        .read(
            negative,
            &points[..3],
            &mut || false,
            &mut |index, actual| {
                assert_eq!(actual, first[index]);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(context.programs.len(), programs);
}

#[test]
fn roll_estimation_keeps_corrected_camera_headroom_and_statistical_inputs() {
    use crate::roll_analysis::{FrameSamples, estimate, estimate_with_model};
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = evaluation::Context::new(&device, evaluation::CachePolicy::default());
    let gate = SubmissionGate::default();
    let matrix = DMat3::from_diagonal(DVec3::new(1.3, 0.8, 1.1));
    // A legal continuous source can exceed its development ceiling after residual correction.
    let base = DVec3::new(0.8, 0.4, 0.2) * 512.;
    assert!(base.max_element() > crate::processing::MAX_SOURCE_GAIN);
    assert!((base * 0.5).max_element() <= crate::processing::MAX_SOURCE_GAIN);

    let film = vec![base.as_vec3().as_dvec3(); 256];
    let pictures: Vec<Vec<_>> = [(301, 0.05, 0.8), (401, 0.07, 0.7)]
        .into_iter()
        .map(|(count, low, high)| {
            (0..count)
                .map(|index| {
                    let transmission = low + (high - low) * index as f64 / (count - 1) as f64;
                    let channels = DVec3::new(
                        transmission.powf(1.05),
                        transmission.powf(0.97),
                        transmission,
                    );

                    (base * channels).as_vec3().as_dvec3()
                })
                .collect()
        })
        .collect();
    let frames: Vec<_> = pictures
        .iter()
        .map(|picture| FrameSamples {
            film: &film,
            picture,
        })
        .collect();
    let current = NegativeInput {
        calibration: Calibration {
            dmin: base,
            dmax: base * 0.02,
            input_floor: 512. / 65535.,
        },
        gamma: 0.6,
        response: Response::film(),
        balance: Balance {
            warmth: 0.3,
            tint: -0.2,
        },
    };
    let reference = estimate(
        &frames,
        current.calibration.input_floor,
        matrix,
        current,
        || false,
    )
    .unwrap()
    .unwrap();
    let mut model = ResponseModel::new(&context, (&queue, &gate));
    let actual = estimate_with_model(
        &frames,
        current.calibration.input_floor,
        matrix,
        current,
        &mut model,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(actual.input.calibration(), reference.input.calibration());
    assert_eq!(actual.frames, reference.frames);
    let actual_balance = actual.input.negative_input().unwrap().balance;
    let reference_balance = reference.input.negative_input().unwrap().balance;

    assert!(
        reference_balance.warmth.abs() + reference_balance.tint.abs() > 0.01,
        "the population must require a nonidentity curve fit"
    );
    assert!(
        (actual_balance.warmth - reference_balance.warmth).abs() < 2e-4,
        "{actual_balance:?} != {reference_balance:?}"
    );
    assert!(
        (actual_balance.tint - reference_balance.tint).abs() < 2e-4,
        "{actual_balance:?} != {reference_balance:?}"
    );
    assert!((actual.roll.exposure - reference.roll.exposure).abs() < 2e-4);
    assert!((actual.roll.wb.warmth - reference.roll.wb.warmth).abs() < 2e-4);
    assert!((actual.roll.wb.tint - reference.roll.wb.tint).abs() < 2e-4);
    for (actual, reference) in actual.photos.iter().zip(&reference.photos) {
        assert!((actual.exposure - reference.exposure).abs() < 2e-4);
    }
}

#[test]
fn response_batches_stop_before_acquisition_or_during_delivery_and_can_be_reused() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = evaluation::Context::new(&device, evaluation::CachePolicy::default());
    let gate = SubmissionGate::default();
    let mut model = ResponseModel::new(&context, (&queue, &gate));
    let points = vec![DVec3::splat(0.03); 10];
    assert!(
        !model
            .read(negative(), &points, &mut || true, &mut |_, _| panic!(
                "cancelled batch"
            ))
            .unwrap()
    );
    assert_eq!(context.programs.len(), 0);
    let stopped = Cell::new(false);
    let mut delivered = 0;
    assert!(
        !model
            .read(negative(), &points, &mut || stopped.get(), &mut |_, _| {
                delivered += 1;
                stopped.set(true);
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(delivered, 1);
    delivered = 0;
    assert!(
        model
            .read(negative(), &points, &mut || false, &mut |_, _| {
                delivered += 1;
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(delivered, 10);
}
