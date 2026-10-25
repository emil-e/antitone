#![cfg(feature = "gpu")]
use antitone::{
    color::Balance,
    gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource},
    processing::{
        Calibration, InputSettings, NegativeInput, PhotoSettings, Pipeline, Response, RollSettings,
    },
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

fn as_input(negative: NegativeInput) -> InputSettings {
    InputSettings::Negative {
        dmin: Some(negative.calibration.dmin),
        dmax: Some(negative.calibration.dmax),
        input_floor: negative.calibration.input_floor,
        response: negative.response,
        gamma: negative.gamma,
        balance: negative.balance,
    }
}

fn camera(code: [u16; 4]) -> DVec3 {
    DVec3::new(code[0] as f64, code[1] as f64, code[2] as f64) / 65535.
}

fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernel: &ProcessingKernel,
    source: &ProcessingSource,
    params: ProcessingParameters,
    n: usize,
) -> Vec<[f32; 4]> {
    let size = n as u64 * 16;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    kernel.encode(device, &mut encoder, source, &output, params);
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

#[test]
fn film_shapes_camera_matrices_balance_and_exposure_match_the_scalar_model() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device).await.unwrap();
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
        let base_code = [12695, 8535, 7802, 0];
        let mut codes = vec![
            base_code,
            [0; 4],
            [1, 2, 2, 0],
            [65535; 4],
            [65535, 0, 0, 0],
            [0, 65535, 0, 0],
            [0, 0, 65535, 0],
            [500, 200, 100, 0],
        ];
        let mut rng = 0x619af501_u32;
        for _ in 0..512 {
            let mut code = [0; 4];
            for c in &mut code[..3] {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                *c = rng as u16;
            }
            codes.push(code);
        }
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
        let mut worst = 0_f64;
        for matrix in matrices {
            let source = ProcessingSource::new(&device, &codes, matrix).unwrap();
            let scans: Vec<_> = codes.iter().map(|c| camera(*c)).collect();
            for case in 0..120 {
                let toe = [1., 1.125, 2.5, 3.875, 4.][case % 5];
                let shoulder = [0., 0.0043210987, 0.02][case / 5 % 3];
                let gamma = [0.4, 0.65, 1.][case / 15 % 3];
                let ev = [-2., -0.731234567, 0., 2.345678901, 3., 4.][case / 7 % 6];
                let cal = Calibration {
                    dmin: DVec3::new(0.21, 0.15, 0.13),
                    dmax: DVec3::new(0.008061, 0.003409, 0.001442),
                    input_floor: 2.1630933622265155e-5,
                };
                let interpretation = negative(
                    cal,
                    Response::Film { toe, shoulder },
                    gamma,
                    balances[case / 13 % 4],
                );
                let roll = RollSettings {
                    wb: balances[case / 19 % 4],
                    exposure: ev / 2.,
                };
                let photo = PhotoSettings {
                    exposure: ev / 2.,
                    wb: balances[case / 17 % 4],
                    ..PhotoSettings::default()
                };
                let params =
                    match ProcessingParameters::positive(interpretation, roll, photo, matrix) {
                        Ok(p) => p,
                        Err(_) => {
                            rejected += 1;
                            continue;
                        }
                    };
                let pipeline =
                    Pipeline::new(as_input(interpretation), roll, photo, matrix).unwrap();
                let gpu = read(&device, &queue, &kernel, &source, params, codes.len());
                accepted += 1;
                for (i, s) in scans.iter().enumerate() {
                    let expected = pipeline.apply(*s).unwrap();
                    let flags = (0..3).fold(0u32, |b, c| b | (u32::from(expected.floored[c]) << c));
                    assert_eq!(gpu[i][3] as u32, flags);
                    // Signed matrix/WB products can cancel within one channel.
                    let allowed = 2e-5 + 2e-4 * expected.rgb.abs().max_element();
                    for c in 0..3 {
                        let ratio = (gpu[i][c] as f64 - expected.rgb[c]).abs() / allowed;
                        worst = worst.max(ratio);
                        assert!(
                            gpu[i][c].is_finite() && ratio <= 1.,
                            "case={case} input={i} channel={c} GPU={:?} CPU={:?} ratio={ratio}",
                            gpu[i],
                            expected.rgb
                        );
                    }
                }
            }
            let cal = Calibration {
                dmin: scans[0],
                dmax: scans[7],
                input_floor: 2.1630933622265155e-5,
            };
            let roll = RollSettings::default();
            let params =
                ProcessingParameters::positive(cal, roll, PhotoSettings::default(), matrix)
                    .unwrap();
            let gpu = read(&device, &queue, &kernel, &source, params, codes.len());
            let pipeline = Pipeline::new(cal, roll, PhotoSettings::default(), matrix).unwrap();
            for i in [0, 7] {
                let expected = pipeline.apply(scans[i]).unwrap().rgb;
                for c in 0..3 {
                    assert!(
                        (gpu[i][c] as f64 - expected[c]).abs() <= 2e-5 + 2e-4 * expected[c].abs()
                    );
                }
            }
        }
        assert!(accepted > 200);
        println!(
            "film GPU: {accepted} accepted / {rejected} domain-or-range rejected; worst tolerance multiple={worst:.9e}"
        );
    });
}

#[test]
fn scalar_valid_floor_anchor_is_rejected_or_stays_finite_with_packed_parameters() {
    let floor = 10_f64.powf(-(0.65 / 0.02 - 1e-5));
    let interpretation = negative(
        Calibration {
            dmin: DVec3::ONE,
            dmax: DVec3::splat(floor),
            input_floor: floor,
        },
        Response::Film {
            toe: 2.5,
            shoulder: 0.02,
        },
        0.65,
        Balance::default(),
    );
    let scalar = antitone::processing::Inversion::new(interpretation).unwrap();
    assert_eq!(scalar.apply(DVec3::splat(floor)).unwrap().rgb, DVec3::ONE);
    let Ok(parameters) = ProcessingParameters::positive(
        interpretation,
        RollSettings::default(),
        PhotoSettings::default(),
        DMat3::IDENTITY,
    ) else {
        return;
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        let source =
            ProcessingSource::new_continuous(&device, &[[0.; 4]], DMat3::IDENTITY).unwrap();
        let actual = read(&device, &queue, &kernel, &source, parameters, 1)[0];
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
        Response::Film {
            toe: 2.5,
            shoulder: 0.02,
        },
        0.1,
        Balance::default(),
    );
    assert!(
        ProcessingParameters::positive(
            interpretation,
            RollSettings::default(),
            PhotoSettings::default(),
            DMat3::IDENTITY
        )
        .is_err()
    );
}
