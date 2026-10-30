#![cfg(feature = "gpu")]
use crate::gpu::camera_fixture::CameraFixture;
use antitone::adjustment_layers::{
    AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
};
use antitone::gpu::ProcessingInput;
use antitone::{
    color::Balance,
    gpu::{ProcessingParameters, ProcessingSource},
    processing::{Calibration, NegativeInput, Response, RollSettings},
};
use glam::{DMat3, DVec3};

fn negative(cal: Calibration, response: Response, gamma: f64, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response,
        gamma,
        balance,
    }
}

fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernel: &CameraFixture,
    source: &ProcessingSource,
    params: &ProcessingInput,
    n: usize,
) -> Vec<[f32; 4]> {
    let size = n as u64 * 16;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    kernel
        .encode_grade(device, &mut encoder, source, &output, params)
        .unwrap();
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    read.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = read.get_mapped_range(..).unwrap();
    let result = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec();
    drop(mapped);
    read.unmap();
    result
}

// Construct sensor inputs from declared native positives by algebraically reversing the
// one-dimensional curve. This fixture has no image assembly, matrix, WB or grading evaluator.
fn sensor_for(negative: NegativeInput, positive: DVec3) -> DVec3 {
    let calibration = negative.calibration;
    let gammas = negative.channel_gammas().unwrap();
    DVec3::from_array(std::array::from_fn(|c| {
        let density = (calibration.dmin[c] / calibration.dmax[c]).ln();
        let toe = negative.response.toe;
        let shoulder = negative.response.shoulder;
        let anchor = toe * density / (gammas[c] - shoulder * density / std::f64::consts::LN_10);
        let exponent = (positive[c].powf(toe) * anchor.exp_m1()).ln_1p();
        let attenuation =
            gammas[c] * exponent / (toe + shoulder * exponent / std::f64::consts::LN_10);
        calibration.dmin[c] * (-attenuation).exp()
    }))
}

#[test]
fn shaped_responses_recover_declared_positives_before_matrix_wb_and_exposure() {
    pollster::block_on(async {
        let adapter = antitone::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let fixture = CameraFixture::new(&device).await.unwrap();
        let matrices = [
            DMat3::from_cols(
                DVec3::new(
                    0.6062051167485374,
                    0.01610845369283179,
                    -0.003058878537448056,
                ),
                DVec3::new(0.38024835632147796, 1.192683246450255, -0.14899261591291027),
                DVec3::new(
                    0.013546559132378455,
                    -0.20879153479206025,
                    1.1520515014561068,
                ),
            ),
            DMat3::IDENTITY,
            DMat3::from_cols(
                DVec3::new(1.7, -0.13, 0.1),
                DVec3::new(-0.32, 1.2, -0.11),
                DVec3::new(0.09, -0.25, 1.4),
            ),
        ];
        let positives = [
            DVec3::ZERO,
            DVec3::ONE,
            DVec3::new(0.125, 0.5, 4.),
            DVec3::new(4., 0.125, 0.5),
            DVec3::new(0.25, 2., 1.),
            DVec3::new(1., 0.25, 2.),
        ];
        let balances = [
            Balance::default(),
            Balance {
                warmth: -1.,
                tint: 1.,
            },
            Balance {
                warmth: 1.,
                tint: -1.,
            },
            Balance {
                warmth: 0.123456789,
                tint: -0.678901234,
            },
        ];
        let mut accepted = 0;
        let mut rejected = 0;
        let mut signed = false;
        let mut hdr = false;
        for matrix in matrices {
            for case in 0..120 {
                let calibration = Calibration {
                    dmin: DVec3::new(0.21, 0.15, 0.13),
                    dmax: DVec3::new(0.008061, 0.003409, 0.001442),
                    input_floor: 2.1630933622265155e-5,
                };
                let interpretation = negative(
                    calibration,
                    Response {
                        toe: [1., 1.125, 2.5, 3.875, 4.][case % 5],
                        shoulder: [0., 0.0043210987, 0.02][case / 5 % 3],
                    },
                    [0.4, 0.65, 1.][case / 15 % 3],
                    balances[case / 13 % 4],
                );
                let exposure = [-2., -0.731234567, 0., 2.345678901, 3., 4.][case / 7 % 6];
                let roll = RollSettings {
                    wb: balances[case / 19 % 4],
                    exposure: exposure / 2.,
                };
                let photo = AdjustmentLayerSettings {
                    wb: balances[case / 17 % 4],
                    exposure: exposure / 2.,
                    ..Default::default()
                };
                let input = match ProcessingInput::new(
                    &device,
                    interpretation,
                    roll,
                    AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(photo)]).unwrap(),
                    matrix,
                    [0, 0, positives.len() as u32 + 2, 1].into(),
                ) {
                    Ok(input) => input,
                    Err(_) => {
                        rejected += 1;
                        continue;
                    }
                };
                let mut sensors: Vec<_> = positives
                    .iter()
                    .map(|p| sensor_for(interpretation, *p))
                    .collect();
                sensors.extend([calibration.dmin * 1.1, DVec3::ZERO]);
                let values: Vec<_> = sensors
                    .iter()
                    .map(|p| (*p * 65535.).as_vec3().extend(0.).to_array())
                    .collect();
                let source = ProcessingSource::new_continuous(&device, &values, matrix).unwrap();
                let actual = read(&device, &queue, &fixture, &source, &input, values.len());
                let transform = antitone::color::wb_matrix(photo.wb).unwrap()
                    * antitone::color::wb_matrix(roll.wb).unwrap()
                    * matrix;
                for (i, expected) in positives.into_iter().enumerate() {
                    let expected = transform * expected * exposure.exp2();
                    signed |= expected.min_element() < 0.;
                    hdr |= expected.max_element() > 1.;
                    assert_eq!(
                        actual[i][3], 0.,
                        "declared positive is above the source floor"
                    );
                    let allowed = 2e-5 + 2e-4 * expected.abs().max_element();
                    for c in 0..3 {
                        assert!(
                            actual[i][c].is_finite()
                                && (actual[i][c] as f64 - expected[c]).abs() <= allowed,
                            "case={case} input={i} channel={c}: {:?} vs {expected:?}",
                            actual[i]
                        );
                    }
                }
                let above_base = actual[positives.len()];
                assert_eq!(above_base[3], 0.);
                if case == 0 && matrix == DMat3::IDENTITY {
                    assert!(
                        above_base[..3].iter().all(|v| *v < 0.),
                        "above-base continuation stays signed"
                    );
                }
                let floored = actual[positives.len() + 1];
                assert_eq!(floored[3], 7.);
                assert!(
                    floored[..3].iter().all(|v| v.is_finite()),
                    "admitted floor response remains finite"
                );
                accepted += 1;
            }
        }
        assert!(accepted > 200, "accepted={accepted} rejected={rejected}");
        assert!(
            signed && hdr,
            "matrix/WB controls must exercise signed and HDR working output"
        );
    });
}

#[test]
fn valid_floor_anchor_is_rejected_or_stays_finite_with_packed_parameters() {
    let floor = 10_f64.powf(-(0.65 / 0.02 - 1e-5));
    let interpretation = negative(
        Calibration {
            dmin: DVec3::ONE,
            dmax: DVec3::splat(floor),
            input_floor: floor,
        },
        Response {
            toe: 2.5,
            shoulder: 0.02,
        },
        0.65,
        Balance::default(),
    );
    interpretation.parameters().unwrap();
    let Ok(_) =
        ProcessingParameters::positive(interpretation, RollSettings::default(), DMat3::IDENTITY)
    else {
        return;
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = CameraFixture::new(&device).await.unwrap();
        let source =
            ProcessingSource::new_continuous(&device, &[[0.; 4]], DMat3::IDENTITY).unwrap();
        let grade = ProcessingInput::new(
            &device,
            interpretation,
            RollSettings::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            DMat3::IDENTITY,
            [0, 0, 1, 1].into(),
        )
        .unwrap();
        let actual = read(&device, &queue, &kernel, &source, &grade, 1)[0];
        assert_eq!(actual[3] as u32, 7);
        for value in &actual[..3] {
            assert!(value.is_finite() && (value - 1.).abs() < 2e-4, "{actual:?}");
        }
    });
}

#[test]
fn response_rejects_a_pole_inside_the_supported_sample_domain() {
    let interpretation = negative(
        Calibration {
            dmin: DVec3::ONE,
            dmax: DVec3::splat(0.01),
            input_floor: 1e-5,
        },
        Response {
            toe: 2.5,
            shoulder: 0.02,
        },
        0.1,
        Balance::default(),
    );
    assert!(
        ProcessingParameters::positive(interpretation, RollSettings::default(), DMat3::IDENTITY)
            .is_err()
    );
}
