#![cfg(all(feature = "gpu", target_os = "macos"))]

use antitone::color::{Balance, ap0_to_ap1};
use antitone::gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource};
use antitone::processing::{Calibration, PhotoSettings, Pipeline, RollSettings};
use glam::{DMat3, DVec3, Vec4};

#[test]
fn metal_processing_matches_the_cpu_model_and_source_bypass() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        let cal = Calibration {
            dmin: DVec3::new(0.75, 0.48, 0.22),
            dmax: DVec3::new(0.003, 0.006, 0.008),
            input_floor: 2e-5,
        };
        let matrix = DMat3::from_cols(
            DVec3::new(1., 0., -0.2),
            DVec3::new(-0.3, 1., 0.),
            DVec3::new(0., 0., 1.5),
        );
        let pixels: Vec<[u16; 4]> = [
            cal.dmin,
            cal.dmax,
            cal.dmin * 1.1,
            cal.dmax / 2.,
            DVec3::new(0., 0.01, 0.02),
            DVec3::ZERO,
            cal.dmin * (1. - 1e-5),
            DVec3::splat(cal.input_floor),
            DVec3::new(0.05, 0.07, 0.02),
        ]
        .into_iter()
        .map(|s| {
            let camera = matrix.inverse() * s;
            [
                (camera.x * 65535.).round() as u16,
                (camera.y * 65535.).round() as u16,
                (camera.z * 65535.).round() as u16,
                1234,
            ]
        })
        .chain([
            [0, 65535, 0, 65535],
            [65535, 0, 0, 65535],
            [0, 0, 65535, 65535],
        ])
        .collect();
        let scans: Vec<_> = pixels
            .iter()
            .map(|p| matrix * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.))
            .collect();
        assert!(scans.iter().any(|s| s.min_element() < 0.));
        assert!(scans.iter().any(|s| s.max_element() > 1.));
        let input = ProcessingSource::new(&device, &pixels, matrix).unwrap();
        assert_eq!(input.byte_len(), (pixels.len() * 8) as u64);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test working pixels"),
            size: (pixels.len() * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test readback"),
            size: output.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut configurations = vec![(None, ProcessingParameters::source())];
        for gamma in [0.1, 0.65, 1.0, 3.0] {
            for balance in [
                Balance::default(),
                Balance {
                    warmth: 0.21,
                    tint: -0.14,
                },
            ] {
                let roll = RollSettings {
                    gamma,
                    balance,
                    exposure: 1.5,
                    ..RollSettings::default()
                };
                let photo = PhotoSettings {
                    exposure: -1.0,
                    wb: Balance {
                        warmth: 0.2,
                        tint: -0.1,
                    },
                };
                let parameters = ProcessingParameters::positive(cal, roll, photo).unwrap();
                configurations.push((Some(Pipeline::new(cal, roll, photo).unwrap()), parameters));
            }
        }
        let attenuated_cal = Calibration {
            input_floor: 1e-6,
            ..cal
        };
        let attenuated_roll = RollSettings {
            gamma: 0.1,
            exposure: -16.0,
            ..Default::default()
        };
        let attenuated_photo = PhotoSettings {
            exposure: -16.0,
            ..Default::default()
        };
        configurations.push((
            Some(Pipeline::new(attenuated_cal, attenuated_roll, attenuated_photo).unwrap()),
            ProcessingParameters::positive(attenuated_cal, attenuated_roll, attenuated_photo)
                .unwrap(),
        ));
        for (pipeline, parameters) in configurations {
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(&device, &mut encoder, &input, &output, parameters);
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                sender.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range().unwrap();
            let actual: &[Vec4] = bytemuck::cast_slice(&mapped);
            for (i, &scan) in scans.iter().enumerate() {
                let (expected, flags) = if let Some(pipeline) = &pipeline {
                    let value = pipeline.apply(scan).unwrap();
                    let flags = value
                        .floored
                        .iter()
                        .enumerate()
                        .fold(0u32, |m, (c, f)| m | (u32::from(*f) << c));
                    (value.rgb, flags)
                } else {
                    (ap0_to_ap1() * scan, 0)
                };
                for c in 0..3 {
                    let gpu = f64::from(actual[i][c]);
                    assert!(
                        gpu.is_finite()
                            && (gpu - expected[c]).abs() <= 2e-5 + 2e-4 * expected[c].abs(),
                        "pixel {i} channel {c}: GPU={gpu} CPU={}",
                        expected[c]
                    );
                }
                assert_eq!(actual[i].w as u32, flags);
            }
            drop(mapped);
            readback.unmap();
        }
    });
}

#[test]
fn gpu_parameters_reject_overflow_instead_of_silent_cpu_fallback() {
    let cal = Calibration {
        dmin: DVec3::ONE,
        dmax: DVec3::splat(0.01),
        input_floor: 1e-6,
    };
    let roll = RollSettings {
        gamma: 0.1,
        balance: Balance {
            warmth: -1.0,
            tint: 1.0,
        },
        exposure: 16.0,
        ..RollSettings::default()
    };
    assert!(
        ProcessingParameters::positive(
            cal,
            roll,
            PhotoSettings {
                exposure: 16.0,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(std::mem::size_of::<ProcessingParameters>(), 224);
}
