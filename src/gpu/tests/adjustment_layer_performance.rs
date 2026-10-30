#![cfg(all(feature = "ui", feature = "render"))]
use crate::gpu::camera_fixture::CameraFixture;
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSnapshot},
    color::Balance,
    geometry::{SourcePoint, SourceRegion},
    gpu::{ProcessingInput, ProcessingSource, SourceGeometry},
    processing::{Calibration, RollSettings},
    rendering::OutputTarget,
    viewer::GpuImage,
};
use glam::{DMat3, DVec3};
use std::{sync::Arc, time::Instant};

const AREA: SourceRegion = SourceRegion {
    x: 12,
    y: 12,
    width: 8192,
    height: 5472,
};

fn layers(count: usize, photographic: bool) -> Vec<AdjustmentLayer> {
    (0..count)
        .map(|i| AdjustmentLayer {
            mask: if i % 2 == 0 {
                AdjustmentLayerCoverage::Linear {
                    start: SourcePoint([900., 800.]),
                    end: SourcePoint([7000., 4400.]),
                }
            } else {
                AdjustmentLayerCoverage::Radial {
                    center: SourcePoint([4100., 2750.]),
                    radii: [3500., 2300.],
                    angle: 0.3,
                    feather: 0.8,
                }
            },
            photo: if photographic {
                AdjustmentLayerSettings {
                    exposure: 0.25,
                    wb: Balance {
                        warmth: 0.005,
                        tint: -0.003,
                    },
                    contrast: 1.01,
                    contrast_colour_mix: 0.25,
                    saturation: 1.02,
                    saturation_density: 0.4,
                    ..Default::default()
                }
            } else {
                AdjustmentLayerSettings {
                    exposure: if i % 2 == 0 { 0.25 } else { -0.125 },
                    ..Default::default()
                }
            },
            enabled: true,
            invert: i % 3 == 0,
            strength: 0.6,
        })
        .collect()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn completion(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encode: impl FnOnce(&mut wgpu::CommandEncoder),
) -> f64 {
    let start = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    encode(&mut encoder);
    let submitted = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: None,
        })
        .unwrap();
    start.elapsed().as_secs_f64() * 1000.
}

#[test]
#[ignore = "bounded synthetic Metal preparation/processing/ACES completion measurements, not presentation latency"]
fn local_stage_cost_at_live_preview_and_refinement_tile_sizes() {
    pollster::block_on(async {
        let instance = antitone::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let quick = std::env::var_os("ANTITONE_ADJUSTMENT_LAYER_PERF_QUICK").is_some();
        let sizes: &[[u32; 2]] = if quick {
            &[[1024, 640]]
        } else {
            &[[1024, 256], [1024, 640], [1024, 1024]]
        };
        let counts: &[usize] = if quick {
            &[0, 2, 16]
        } else {
            &[0, 1, 2, 3, 4, 8, 16]
        };
        let samples = if quick { 9 } else { 3 };
        let warmups = if quick { 2 } else { 1 };
        println!(
            "local_performance adapter={:?} samples={samples} warmups={warmups} timing=encode_submit_gpu_completion excludes=presentation_upload_reconstruction",
            adapter.get_info()
        );
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = CameraFixture::new(&device).await.unwrap();
        let renderer = antitone::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let calibration = Calibration {
            dmin: DVec3::new(0.19, 0.14, 0.12),
            dmax: DVec3::new(0.008, 0.004, 0.002),
            input_floor: 1. / 65535.,
        };
        let roll = RollSettings::default();
        let global = AdjustmentLayerSettings::default();
        for &size in sizes {
            let n = size[0] * size[1];
            let pixels: Vec<_> = (0..n)
                .map(|i| {
                    [
                        i.wrapping_mul(1871) as u16,
                        i.wrapping_mul(7919) as u16,
                        i.wrapping_mul(3413) as u16,
                        0,
                    ]
                })
                .collect();
            let region = if size[1] == 256 {
                // A full-resolution centre tile, not a whole-source sparse proxy.
                [
                    AREA.x + AREA.width / 2 - size[0] / 2,
                    AREA.y + AREA.height / 2 - size[1] / 2,
                    size[0],
                    size[1],
                ]
            } else {
                [AREA.x, AREA.y, AREA.width, AREA.height]
            };
            let geometry = SourceGeometry::new(region, size).unwrap();
            println!("local_performance size={size:?} source_region={region:?}");
            let mut source = ProcessingSource::new(&device, &pixels, DMat3::IDENTITY).unwrap();
            source.set_geometry(geometry).unwrap();
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("performance working output"),
                size: u64::from(n) * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let mut image = GpuImage::new_shared(
                &renderer,
                &device,
                size[0],
                size[1],
                &pixels,
                DMat3::IDENTITY,
                OutputTarget::Srgb,
            )
            .await
            .unwrap();
            image.set_geometry(geometry).unwrap();
            assert_eq!(source.byte_len(), u64::from(n) * 8);
            assert_eq!(
                image.processing_byte_len(),
                (
                    u64::from(n) * 8,
                    u64::from(n) * 16 + std::mem::size_of::<[u32; 4]>() as u64
                )
            );
            for photographic in [false, true] {
                for &count in counts {
                    let mut preparation = Vec::new();
                    let mut prepared = None;
                    let mut unsupported = None;
                    for _ in 0..3 {
                        let start = Instant::now();
                        let result = ProcessingInput::new(
                            &device,
                            calibration,
                            roll,
                            AdjustmentLayerSnapshot::new(
                                std::iter::once(
                                    antitone::adjustment_layers::AdjustmentLayer::global(global),
                                )
                                .chain(layers(count, photographic))
                                .collect(),
                            )
                            .unwrap(),
                            DMat3::IDENTITY,
                            AREA,
                        )
                        .map(Arc::new);
                        match result {
                            Ok(grade) => {
                                prepared = Some(grade);
                                preparation.push(start.elapsed().as_secs_f64() * 1000.);
                            }
                            Err(error) => {
                                unsupported = Some(error);
                                break;
                            }
                        }
                    }
                    if let Some(error) = unsupported {
                        println!(
                            "local_performance size={size:?} photographic={photographic} count={count} unsupported={error}"
                        );
                        continue;
                    }
                    let prepared = prepared.unwrap();
                    let input = ProcessingInput::from(prepared.clone());
                    let encode = |encoder: &mut wgpu::CommandEncoder| {
                        kernel
                            .encode_grade(&device, encoder, &source, &output, &prepared)
                            .unwrap();
                    };
                    for _ in 0..warmups {
                        completion(&device, &queue, encode);
                    }
                    let processing = (0..samples)
                        .map(|_| completion(&device, &queue, encode))
                        .collect();
                    for _ in 0..warmups {
                        completion(&device, &queue, |encoder| {
                            image.encode_processing(&device, encoder, &input).unwrap()
                        });
                    }
                    let rendered = (0..samples)
                        .map(|_| {
                            completion(&device, &queue, |encoder| {
                                image.encode_processing(&device, encoder, &input).unwrap()
                            })
                        })
                        .collect();
                    println!(
                        "local_performance size={size:?} photographic={photographic} count={count} cpu_resource_prepare_median_ms={:.3} processing_completion_median_ms={:.3} processing_aces_completion_median_ms={:.3} source_bytes={} working_bytes={} grade_bytes={}",
                        median(preparation),
                        median(processing),
                        median(rendered),
                        source.byte_len(),
                        output.size(),
                        prepared.byte_len()
                    );
                }
            }
        }
    });
}
