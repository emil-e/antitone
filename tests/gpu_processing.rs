#![cfg(all(feature = "gpu", target_os = "macos"))]
use antitone::adjustment_layers::{
    AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
};
use antitone::{geometry::SourceRegion, gpu::GpuPipeline};

use antitone::color::Balance;
use antitone::gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource};
use antitone::processing::{
    Calibration, DEFAULT_GAMMA, InputSettings, NegativeInput, Response, RollSettings,
};
use glam::{DMat3, DVec3, Vec4};
#[path = "support/photographic_reference.rs"]
mod reference;

fn negative(cal: Calibration, gamma: f64, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response: Response::film(),
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
            let camera = s;
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
            .map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.)
            .collect();
        assert!(scans.iter().any(|s| (matrix * *s).min_element() < 0.));
        assert!(scans.iter().any(|s| (matrix * *s).max_element() > 1.));
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
        let mut configurations = vec![(None, None)];
        for gamma in [0.1, DEFAULT_GAMMA, 0.65, 1.0, 3.0] {
            for balance in [
                Balance::default(),
                Balance {
                    warmth: 0.21,
                    tint: -0.14,
                },
            ] {
                let interpretation = negative(cal, gamma, balance);
                let roll = RollSettings {
                    exposure: 1.5,
                    ..RollSettings::default()
                };
                let photo = AdjustmentLayerSettings {
                    exposure: -1.0,
                    wb: Balance {
                        warmth: 0.2,
                        tint: -0.1,
                    },
                    ..AdjustmentLayerSettings::default()
                };
                let parameters = AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(photo)])
                    .and_then(|snapshot| {
                        GpuPipeline::new(
                            &device,
                            interpretation,
                            roll,
                            snapshot,
                            matrix,
                            SourceRegion {
                                x: 0,
                                y: 0,
                                width: pixels.len() as u32,
                                height: 1,
                            },
                        )
                    })
                    .map(Some)
                    .unwrap();
                configurations.push((
                    Some(
                        antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                            antitone::adjustment_layers::AdjustmentLayer::global(photo),
                        ])
                        .and_then(|layers| {
                            antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                                as_input(interpretation),
                                roll,
                                layers,
                                matrix,
                            )
                        })
                        .unwrap(),
                    ),
                    parameters,
                ));
            }
        }
        let attenuated_cal = Calibration {
            input_floor: 1e-6,
            ..cal
        };
        let attenuated = negative(attenuated_cal, 0.1, Balance::default());
        let attenuated_roll = RollSettings {
            exposure: -16.0,
            ..Default::default()
        };
        let attenuated_photo = AdjustmentLayerSettings {
            exposure: -16.0,
            ..Default::default()
        };
        configurations.push((
            Some(
                antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                    antitone::adjustment_layers::AdjustmentLayer::global(attenuated_photo),
                ])
                .and_then(|layers| {
                    antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                        as_input(attenuated),
                        attenuated_roll,
                        layers,
                        matrix,
                    )
                })
                .unwrap(),
            ),
            AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(attenuated_photo)])
                .and_then(|snapshot| {
                    GpuPipeline::new(
                        &device,
                        attenuated,
                        attenuated_roll,
                        snapshot,
                        matrix,
                        SourceRegion {
                            x: 0,
                            y: 0,
                            width: pixels.len() as u32,
                            height: 1,
                        },
                    )
                })
                .map(Some)
                .unwrap(),
        ));
        for contrast in [0.4, 0.5, 0.8, 1.2, 1.5, 1.6] {
            for pivot in [0.01, 0.18, 1.] {
                for colour in [0., 0.5, 1.] {
                    for saturation in [0., 0.7, 1.3, 2.] {
                        for exposure in [-2., 0., 2.] {
                            let roll = RollSettings {
                                wb: Balance {
                                    warmth: 0.14,
                                    tint: -0.18,
                                },
                                ..RollSettings::default()
                            };
                            let photo = AdjustmentLayerSettings {
                                contrast,
                                pivot,
                                contrast_colour_mix: colour,
                                saturation,
                                saturation_density: 1. - colour,
                                exposure,
                                wb: Balance {
                                    warmth: 0.2,
                                    tint: -0.1,
                                },
                            };
                            let interpretation = negative(cal, DEFAULT_GAMMA, Balance::default());
                            configurations.push((
                                Some(
                                    antitone::adjustment_layers::AdjustmentLayerSnapshot::new(
                                        vec![antitone::adjustment_layers::AdjustmentLayer::global(
                                            photo,
                                        )],
                                    )
                                    .and_then(|layers| {
                                        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                                            as_input(interpretation),
                                            roll,
                                            layers,
                                            matrix,
                                        )
                                    })
                                    .unwrap(),
                                ),
                                AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(photo)])
                                    .and_then(|snapshot| {
                                        GpuPipeline::new(
                                            &device,
                                            interpretation,
                                            roll,
                                            snapshot,
                                            matrix,
                                            SourceRegion {
                                                x: 0,
                                                y: 0,
                                                width: pixels.len() as u32,
                                                height: 1,
                                            },
                                        )
                                    })
                                    .map(Some)
                                    .unwrap(),
                            ));
                        }
                    }
                }
            }
        }
        for (pipeline, parameters) in configurations {
            let mut encoder = device.create_command_encoder(&Default::default());
            match parameters {
                Some(grade) => kernel
                    .encode_grade(&device, &mut encoder, &input, &output, &grade)
                    .unwrap(),
                None => kernel.encode(
                    &device,
                    &mut encoder,
                    &input,
                    &output,
                    ProcessingParameters::source(),
                ),
            }
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
                    let value = pipeline.apply(scan, [0, 0]).unwrap();
                    let flags = value
                        .floored
                        .iter()
                        .enumerate()
                        .fold(0u32, |m, (c, f)| m | (u32::from(*f) << c));
                    (value.rgb, flags)
                } else {
                    (matrix * scan, 0)
                };
                for c in 0..3 {
                    let gpu = f64::from(actual[i][c]);
                    assert!(
                        gpu.is_finite()
                            && (gpu - expected[c]).abs()
                                <= 2e-5 + 2e-4 * expected.abs().max_element(),
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
fn metal_digital_processing_matches_the_cpu_model() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        let matrix = DMat3::from_cols(
            DVec3::new(1., 0., -0.2),
            DVec3::new(-0.3, 1., 0.),
            DVec3::new(0., 0., 1.5),
        );
        let pixels: Vec<[u16; 4]> = [
            DVec3::splat(0.18),
            DVec3::new(0.02, 0.5, 0.9),
            DVec3::ZERO,
            DVec3::splat(1.0),
            DVec3::new(0.9, 0.1, 0.05),
        ]
        .into_iter()
        .map(|s| {
            [
                (s.x * 65535.).round() as u16,
                (s.y * 65535.).round() as u16,
                (s.z * 65535.).round() as u16,
                1234,
            ]
        })
        .chain([[0, 65535, 0, 65535], [65535, 0, 0, 65535]])
        .collect();
        let scans: Vec<_> = pixels
            .iter()
            .map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.)
            .collect();
        // Digital interprets as already-positive; the camera matrix yields signed/HDR values.
        assert!(scans.iter().any(|s| (matrix * *s).min_element() < 0.));
        assert!(scans.iter().any(|s| (matrix * *s).max_element() > 1.));
        let input = ProcessingSource::new(&device, &pixels, matrix).unwrap();
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
        let mut configurations = Vec::new();
        for exposure in [-16., 0., 1.5, 16.] {
            for wb in [
                Balance::default(),
                Balance {
                    warmth: 0.2,
                    tint: -0.1,
                },
            ] {
                for contrast in [0.5, 1., 1.5] {
                    for saturation in [0., 1.3] {
                        let roll = RollSettings { wb, exposure };
                        let photo = AdjustmentLayerSettings {
                            contrast,
                            saturation,
                            exposure: -1.,
                            wb: Balance {
                                warmth: 0.1,
                                tint: 0.05,
                            },
                            ..AdjustmentLayerSettings::default()
                        };
                        configurations.push((
                            antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                                antitone::adjustment_layers::AdjustmentLayer::global(photo),
                            ])
                            .and_then(|layers| {
                                antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                                    InputSettings::Digital,
                                    roll,
                                    layers,
                                    matrix,
                                )
                            })
                            .unwrap(),
                            AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(photo)])
                                .and_then(|snapshot| {
                                    GpuPipeline::new(
                                        &device,
                                        InputSettings::Digital,
                                        roll,
                                        snapshot,
                                        matrix,
                                        SourceRegion {
                                            x: 0,
                                            y: 0,
                                            width: pixels.len() as u32,
                                            height: 1,
                                        },
                                    )
                                })
                                .map(Some)
                                .unwrap(),
                        ));
                    }
                }
            }
        }
        for (pipeline, parameters) in configurations {
            let mut encoder = device.create_command_encoder(&Default::default());
            match parameters {
                Some(grade) => kernel
                    .encode_grade(&device, &mut encoder, &input, &output, &grade)
                    .unwrap(),
                None => kernel.encode(
                    &device,
                    &mut encoder,
                    &input,
                    &output,
                    ProcessingParameters::source(),
                ),
            }
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
                let value = pipeline.apply(scan, [0, 0]).unwrap();
                // Digital never floors (the source is already a positive).
                assert_eq!(value.floored, [false; 3]);
                for (c, expected) in value.rgb.to_array().into_iter().enumerate() {
                    let gpu = f64::from(actual[i][c]);
                    assert!(
                        gpu.is_finite() && (gpu - expected).abs() <= 2e-5 + 2e-4 * expected.abs(),
                        "pixel {i} channel {c}: GPU={gpu} CPU={expected}",
                    );
                }
                assert_eq!(actual[i].w as u32, 0);
            }
            drop(mapped);
            readback.unmap();
        }
    });
}

#[test]
fn metal_creative_controls_preserve_signed_hdr_model_samples() {
    use wgpu::util::DeviceExt;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct photographic primitive gate"),
            source: wgpu::ShaderSource::Wgsl(
                [
                    include_str!("../src/shaders/camera_conversion.wgsl"),
                    include_str!("../src/shaders/flat_field.wgsl"),
                    include_str!("../src/shaders/film_response.wgsl"),
                    include_str!("../src/shaders/film_inversion_export.wgsl"),
                    include_str!("../src/shaders/photographic.wgsl"),
                    include_str!("../src/shaders/adjustment_layers.wgsl"),
                    include_str!("../src/shaders/photographic_colour.wgsl"),
                    include_str!("../src/shaders/photographic_export.wgsl"),
                    include_str!("../src/shaders/processing.wgsl"),
                    r#"
struct PrimitiveParameters { amounts: vec4f, density: vec4f }
@group(0) @binding(2) var<uniform> primitive: PrimitiveParameters;
@group(0) @binding(8) var<storage, read> grade_input: array<vec4f>;
@group(0) @binding(9) var<storage, read_write> grade_output: array<vec4f>;
@compute @workgroup_size(64)
fn grade_test(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&grade_input) { return; }
    let input = grade_input[id.x];
    let rgb = colour_apply(array<f32, 3>(input.x, input.y, input.z),
        primitive.amounts.x, primitive.amounts.y, primitive.amounts.z, primitive.amounts.w, primitive.density.x);
    grade_output[id.x] = finish_pixel(ProcessedPixel(rgb, 0u));
}
"#,
                ]
                .join("\n")
                .into(),
            ),
        });
        let kernel = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("direct creative primitives"),
            layout: None,
            module: &module,
            entry_point: Some("grade_test"),
            compilation_options: Default::default(),
            cache: None,
        });
        let inputs = [
            DVec3::ZERO,
            DVec3::splat(-1.),
            DVec3::splat(0.0078125),
            DVec3::splat(0.18),
            DVec3::new(4., -0.02, 0.5),
            DVec3::new(0.25, 0.5, 1.),
        ];
        let values: Vec<_> = inputs.iter().map(|v| v.as_vec3().extend(0.)).collect();
        let storage = |label, values: &[Vec4]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(values),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let input = storage("signed/HDR AP1 model samples", &values);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("creative test output"),
            size: inputs.len() as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("creative primitive readback"),
            size: output.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for contrast in [0.4_f64, 0.5, 0.8, 1., 1.2, 1.5, 1.6] {
            let toe_pivot = if contrast < 1. {
                let qp = (0.155251141552511 - contrast * 0.0729055341958355) / (1. - contrast);
                (qp * 17.52 - 9.72).exp2()
            } else {
                0.18
            };
            for pivot in [0.01, 0.18, 1., toe_pivot] {
                for mix in [0., 0.5, 1.] {
                    for saturation in [0., 1., 2.] {
                        let photo = AdjustmentLayerSettings {
                            contrast,
                            pivot,
                            contrast_colour_mix: mix,
                            saturation,
                            saturation_density: mix,
                            ..AdjustmentLayerSettings::default()
                        };
                        let parameters = [
                            Vec4::new(
                                (photo.contrast - 1.) as f32,
                                reference::encode(photo.pivot) as f32 - 0.072_905_53_f32,
                                photo.contrast_colour_mix as f32,
                                (photo.saturation - 1.) as f32,
                            ),
                            Vec4::new(photo.saturation_density as f32, 0., 0., 0.),
                        ];
                        let uniform =
                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("creative parameters"),
                                contents: bytemuck::bytes_of(&parameters),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                        let entries =
                            [(2, &uniform), (8, &input), (9, &output)].map(|(binding, buffer)| {
                                wgpu::BindGroupEntry {
                                    binding,
                                    resource: buffer.as_entire_binding(),
                                }
                            });
                        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("creative test bindings"),
                            layout: &kernel.get_bind_group_layout(0),
                            entries: &entries,
                        });
                        let mut encoder = device.create_command_encoder(&Default::default());
                        {
                            let mut pass = encoder.begin_compute_pass(&Default::default());
                            pass.set_pipeline(&kernel);
                            pass.set_bind_group(0, &bind, &[]);
                            pass.dispatch_workgroups(1, 1, 1);
                        }
                        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
                        queue.submit([encoder.finish()]);
                        let slice = readback.slice(..);
                        let (send, receive) = std::sync::mpsc::channel();
                        slice.map_async(wgpu::MapMode::Read, move |result| {
                            send.send(result).unwrap()
                        });
                        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                        receive.recv().unwrap().unwrap();
                        let data = slice.get_mapped_range().unwrap();
                        let actual: &[Vec4] = bytemuck::cast_slice(&data);
                        for (i, input) in inputs.iter().enumerate() {
                            let contrasted = reference::blend(
                                reference::contrast(
                                    *input,
                                    photo.contrast,
                                    photo.pivot,
                                    reference::Contrast::Rgb,
                                )
                                .unwrap(),
                                reference::contrast(
                                    *input,
                                    photo.contrast,
                                    photo.pivot,
                                    reference::Contrast::BlackOffsetRatios,
                                )
                                .unwrap(),
                                photo.contrast_colour_mix,
                            )
                            .unwrap();
                            let expected = reference::blend(
                                reference::saturation(
                                    contrasted,
                                    photo.saturation,
                                    reference::Saturation::ConstantY,
                                )
                                .unwrap(),
                                reference::saturation(
                                    contrasted,
                                    photo.saturation,
                                    reference::Saturation::Maximum,
                                )
                                .unwrap(),
                                photo.saturation_density,
                            )
                            .unwrap();
                            let error = (actual[i].truncate().as_dvec3() - expected).abs();
                            let allowed = DVec3::splat(2e-5) + expected.abs() * 2e-4;
                            assert!(
                                error.cmple(allowed).all(),
                                "{photo:?}, input={input:?}, sample={i}, error={error:?}, allowed={allowed:?}"
                            );
                        }
                        drop(data);
                        readback.unmap();
                    }
                }
            }
        }
    });
}

#[test]
fn layer_range_preflight_covers_creative_stages() {
    let cal = Calibration {
        dmin: DVec3::splat(0.5),
        dmax: DVec3::splat(0.01),
        input_floor: 1. / 65535.,
    };
    let extreme = negative(cal, 0.1, Balance::default());
    let limits = wgpu::Limits::default();
    let area = SourceRegion {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let valid = AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(
        AdjustmentLayerSettings::default(),
    )])
    .unwrap();
    assert!(
        GpuPipeline::validate_limits(
            &limits,
            extreme,
            RollSettings::default(),
            &valid,
            DMat3::IDENTITY,
            area
        )
        .is_ok()
    );
    let overflowing =
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 1.5,
            ..Default::default()
        })])
        .unwrap();
    assert!(
        GpuPipeline::validate_limits(
            &limits,
            extreme,
            RollSettings::default(),
            &overflowing,
            DMat3::IDENTITY,
            area
        )
        .is_err()
    );
}

#[test]
fn gpu_parameters_reject_overflow_instead_of_silent_cpu_fallback() {
    let cal = Calibration {
        dmin: DVec3::ONE,
        dmax: DVec3::splat(0.01),
        input_floor: 1e-6,
    };
    let interpretation = negative(
        cal,
        0.1,
        Balance {
            warmth: -1.0,
            tint: 1.0,
        },
    );
    let roll = RollSettings {
        exposure: 16.0,
        ..RollSettings::default()
    };
    assert!(
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: 16.0,
            ..Default::default()
        })])
        .and_then(|snapshot| GpuPipeline::validate_limits(
            &wgpu::Limits::default(),
            interpretation,
            roll,
            &snapshot,
            DMat3::IDENTITY,
            SourceRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        ))
        .is_err()
    );
    assert_eq!(std::mem::size_of::<ProcessingParameters>(), 128);
}
