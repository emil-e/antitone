use super::*;
use crate::gpu::{
    reuse_tests::{exact, read_words},
    *,
};
use crate::{
    color::Balance,
    geometry::{SourcePoint, SourceRegion},
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot},
    processing::{Calibration, PhotoSettings, Response, RollSettings},
};
use glam::{DMat3, DVec3};
use std::sync::Arc;

async fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    adapter.request_device(&Default::default()).await.unwrap()
}

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    }
}

fn matrix() -> DMat3 {
    DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    )
}

fn output(device: &wgpu::Device, pixels: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: pixels as u64 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn pixels() -> Vec<[u16; 4]> {
    (0u32..65536)
        .map(|i| {
            [
                i.wrapping_mul(1871) as u16,
                i.wrapping_mul(7919) as u16,
                i.wrapping_mul(3413) as u16,
                65535,
            ]
        })
        .collect()
}

fn encode(
    device: &wgpu::Device,
    kernel: &ProcessingKernel,
    source: &ProcessingSource,
    output: &wgpu::Buffer,
    parameters: ProcessingParameters,
) -> wgpu::CommandBuffer {
    let mut encoder = device.create_command_encoder(&Default::default());
    kernel.encode(device, &mut encoder, source, output, parameters);
    encoder.finish()
}

fn compare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    fused: &ProcessingKernel,
    source: &ProcessingSource,
    actual: &wgpu::Buffer,
    parameters: ProcessingParameters,
) {
    let expected = output(device, source.pixels as usize);
    queue.submit([encode(device, fused, source, &expected, parameters)]);
    exact(
        "production-response",
        "fused-reference",
        &read_words(device, queue, &expected),
        &read_words(device, queue, actual),
    );
}

#[test]
fn response_initialization_uses_the_hosted_submission_gate() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let submissions = SubmissionGate::default();
        let kernel = ProcessingKernel::new_with_submissions(&device, &queue, submissions.clone())
            .await
            .unwrap();
        let source = ProcessingSource::new(&device, &[[4000, 3000, 2000, 0]], matrix()).unwrap();
        let out = output(&device, 1);
        let parameters = ProcessingParameters::positive(
            calibration(),
            RollSettings::new_grade(),
            PhotoSettings::default(),
            matrix(),
        )
        .unwrap();
        let (ready, started) = std::sync::mpsc::channel();
        let (finished, encoded) = std::sync::mpsc::channel();
        let worker_device = device.clone();
        let worker = submissions.configure(|| {
            let worker = std::thread::spawn(move || {
                ready.send(()).unwrap();
                finished
                    .send(encode(&worker_device, &kernel, &source, &out, parameters))
                    .unwrap();
            });
            started
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert!(matches!(
                encoded.recv_timeout(std::time::Duration::from_millis(100)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            worker
        });
        let commands = encoded
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        worker.join().unwrap();
        let index = submissions.submit(&queue, [commands]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: None,
            })
            .unwrap();
    });
}

#[test]
fn response_cache_keys_reuse_and_bypasses_do_not_allocate_tables() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
        let cache = kernel.responses.as_ref().unwrap();
        let mut source = ProcessingSource::new(&device, &[[1, 0, 65535, 42]], matrix()).unwrap();
        let geometry = SourceGeometry::new([0, 0, 1, 1], [1, 1]).unwrap();
        source.set_geometry(geometry).unwrap();
        let out = output(&device, 1);
        drop(encode(
            &device,
            &kernel,
            &source,
            &out,
            ProcessingParameters::source_inspection(2.).unwrap(),
        ));
        assert!(cache.latest.lock().unwrap().is_none());
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [0, 0, 1, 1],
                [2, 2],
                vec![DVec3::splat(1.1); 4],
            )
            .unwrap(),
        );
        source
            .set_flat_field(
                Some(Arc::new(GpuFlatField::new(&device, field.clone()).unwrap())),
                geometry,
            )
            .unwrap();
        let c = field.processing_calibration(calibration()).unwrap();
        let corrected = ProcessingParameters::positive(
            c,
            RollSettings::new_grade(),
            PhotoSettings::default(),
            matrix(),
        )
        .unwrap();
        drop(encode(&device, &kernel, &source, &out, corrected));
        assert!(cache.latest.lock().unwrap().is_none());
        source.set_flat_field(None, geometry).unwrap();
        let p = ProcessingParameters::positive(
            calibration(),
            RollSettings::new_grade(),
            PhotoSettings::default(),
            matrix(),
        )
        .unwrap();
        drop(encode(&device, &kernel, &source, &out, p));
        let first = cache
            .latest
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .buffer
            .clone();
        assert_eq!(first.size(), BYTE_LEN);
        source.write(&queue, &[[2, 65535, 1234, 0]]).unwrap();
        for photo in [
            PhotoSettings {
                wb: Balance {
                    warmth: 0.005,
                    tint: -0.003,
                },
                ..Default::default()
            },
            PhotoSettings {
                contrast: 1.1,
                saturation: 1.2,
                ..Default::default()
            },
        ] {
            let p = ProcessingParameters::positive(
                calibration(),
                RollSettings::new_grade(),
                photo,
                matrix(),
            )
            .unwrap();
            drop(encode(&device, &kernel, &source, &out, p));
            assert_eq!(cache.latest.lock().unwrap().as_ref().unwrap().buffer, first);
        }
        let roll_wb = ProcessingParameters::positive(
            calibration(),
            RollSettings {
                wb: Balance {
                    warmth: 0.23,
                    tint: -0.17,
                },
                ..RollSettings::new_grade()
            },
            PhotoSettings::default(),
            matrix(),
        )
        .unwrap();
        drop(encode(&device, &kernel, &source, &out, roll_wb));
        assert_eq!(cache.latest.lock().unwrap().as_ref().unwrap().buffer, first);
        let other = ProcessingSource::new(&device, &[[1, 2, 3, 0]], DMat3::IDENTITY).unwrap();
        let p_other = ProcessingParameters::positive(
            calibration(),
            RollSettings::new_grade(),
            PhotoSettings::default(),
            DMat3::IDENTITY,
        )
        .unwrap();
        drop(encode(&device, &kernel, &other, &out, p_other));
        assert_eq!(cache.latest.lock().unwrap().as_ref().unwrap().buffer, first);
        for exposure in [1., 2., 3., 4.] {
            let changed = ProcessingParameters::positive(
                calibration(),
                RollSettings::new_grade(),
                PhotoSettings {
                    exposure,
                    ..Default::default()
                },
                matrix(),
            )
            .unwrap();
            drop(encode(&device, &kernel, &source, &out, changed));
            let latest = cache.latest.lock().unwrap();
            let latest = latest.as_ref().unwrap();
            assert_ne!(latest.buffer, first);
            assert_eq!(latest.key, ResponseKey::new(changed));
            assert_eq!(latest.buffer.size(), BYTE_LEN);
        }
        let latest = cache
            .latest
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .buffer
            .clone();
        let wrong_area = SourceRegion::from([10, 10, 1, 1]);
        let grade = GpuPhotoGrade::new(
            &device,
            calibration(),
            RollSettings::new_grade(),
            PhotoGradeSnapshot::new(
                PhotoSettings::default(),
                vec![LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([10., 10.]),
                        end: SourcePoint([11., 11.]),
                    },
                    photo: PhotoSettings {
                        exposure: 0.25,
                        ..Default::default()
                    },
                    strength: 1.,
                    enabled: true,
                    invert: false,
                }],
            )
            .unwrap(),
            matrix(),
            wrong_area,
        )
        .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            kernel
                .encode_grade(&device, &mut encoder, &source, &out, &grade)
                .is_err()
        );
        assert_eq!(
            cache.latest.lock().unwrap().as_ref().unwrap().buffer,
            latest
        );
    });
}

#[test]
fn immutable_response_initialization_survives_abandonment_reordering_and_workers() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let kernel = Arc::new(ProcessingKernel::new(&device, &queue).await.unwrap());
        let fused = ProcessingKernel::new_fused(&device, &queue).await.unwrap();
        let pixels = pixels();
        let source = Arc::new(ProcessingSource::new(&device, &pixels, matrix()).unwrap());
        let parameters = |exposure| {
            ProcessingParameters::positive(
                calibration(),
                RollSettings::new_grade(),
                PhotoSettings {
                    exposure,
                    ..Default::default()
                },
                matrix(),
            )
            .unwrap()
        };
        let out = output(&device, pixels.len());
        drop(encode(&device, &kernel, &source, &out, parameters(0.)));
        queue.submit([encode(&device, &kernel, &source, &out, parameters(0.))]);
        compare(&device, &queue, &fused, &source, &out, parameters(0.));
        let a = output(&device, pixels.len());
        let b = output(&device, pixels.len());
        let command_a = encode(&device, &kernel, &source, &a, parameters(-2.));
        let command_b = encode(&device, &kernel, &source, &b, parameters(3.));
        queue.submit([command_b, command_a]);
        compare(&device, &queue, &fused, &source, &a, parameters(-2.));
        compare(&device, &queue, &fused, &source, &b, parameters(3.));
        let jobs: Vec<_> = (0..4)
            .map(|i| {
                let kernel = kernel.clone();
                let source = source.clone();
                let device = device.clone();
                std::thread::spawn(move || {
                    let p = ProcessingParameters::positive(
                        calibration(),
                        RollSettings::new_grade(),
                        PhotoSettings {
                            exposure: f64::from(i) / 3.,
                            ..Default::default()
                        },
                        matrix(),
                    )
                    .unwrap();
                    let out = output(&device, source.pixels as usize);
                    let command = encode(&device, &kernel, &source, &out, p);
                    (command, out, p)
                })
            })
            .collect();
        let mut outputs = Vec::new();
        let commands: Vec<_> = jobs
            .into_iter()
            .rev()
            .map(|job| {
                let (command, out, p) = job.join().unwrap();
                outputs.push((out, p));
                command
            })
            .collect();
        queue.submit(commands);
        for (out, p) in outputs {
            compare(&device, &queue, &fused, &source, &out, p);
        }
    });
}

#[test]
fn production_lookup_preserves_all_integer_codes_and_local_stage_bits() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
        let fused = ProcessingKernel::new_fused(&device, &queue).await.unwrap();
        let pixels = pixels();
        let area = SourceRegion::from([12, 12, 8192, 5472]);
        let mut source = ProcessingSource::new(&device, &pixels, matrix()).unwrap();
        source
            .set_geometry(
                SourceGeometry::new([area.x, area.y, area.width, area.height], [256, 256]).unwrap(),
            )
            .unwrap();
        let out = output(&device, pixels.len());
        let expected = output(&device, pixels.len());
        for roll in [
            RollSettings::default(),
            RollSettings::new_grade(),
            RollSettings {
                response: Response::Film {
                    toe: 3.,
                    shoulder: 0.01,
                },
                gamma: 1.2,
                ..RollSettings::new_grade()
            },
        ] {
            for exposure in [-16., 0., 16.] {
                let photo = PhotoSettings {
                    exposure,
                    contrast: 0.4,
                    pivot: 0.03,
                    contrast_colour_mix: 1.,
                    ..Default::default()
                };
                let locals = vec![LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([900., 800.]),
                        end: SourcePoint([7000., 4400.]),
                    },
                    photo: PhotoSettings {
                        exposure: 0.25,
                        saturation: 1.02,
                        saturation_density: 0.4,
                        ..Default::default()
                    },
                    strength: 0.6,
                    enabled: true,
                    invert: false,
                }];
                let grade = GpuPhotoGrade::new(
                    &device,
                    calibration(),
                    roll,
                    PhotoGradeSnapshot::new(photo, locals).unwrap(),
                    matrix(),
                    area,
                )
                .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                kernel
                    .encode_grade(&device, &mut encoder, &source, &out, &grade)
                    .unwrap();
                fused
                    .encode_grade(&device, &mut encoder, &source, &expected, &grade)
                    .unwrap();
                queue.submit([encoder.finish()]);
                exact(
                    "production-all-codes",
                    "local-stack",
                    &read_words(&device, &queue, &expected),
                    &read_words(&device, &queue, &out),
                );
            }
        }
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [12, 12, 8192, 5472],
                [2, 2],
                vec![
                    DVec3::new(1.1, 0.9, 1.2),
                    DVec3::new(1.2, 1.1, 0.88),
                    DVec3::ONE,
                    DVec3::splat(1.05),
                ],
            )
            .unwrap(),
        );
        let p = ProcessingParameters::positive(
            field.processing_calibration(calibration()).unwrap(),
            RollSettings::new_grade(),
            PhotoSettings::default(),
            matrix(),
        )
        .unwrap();
        source
            .set_flat_field(
                Some(Arc::new(GpuFlatField::new(&device, field).unwrap())),
                source.geometry.unwrap(),
            )
            .unwrap();
        queue.submit([encode(&device, &kernel, &source, &out, p)]);
        compare(&device, &queue, &fused, &source, &out, p);
        let p = ProcessingParameters::source_inspection(2.).unwrap();
        queue.submit([encode(&device, &kernel, &source, &out, p)]);
        compare(&device, &queue, &fused, &source, &out, p);
        let cache = kernel.responses.as_ref().unwrap().latest.lock().unwrap();
        assert_eq!(cache.as_ref().unwrap().buffer.size(), BYTE_LEN);
        assert_ne!(cache.as_ref().unwrap().key, ResponseKey::new(p));
    });
}
