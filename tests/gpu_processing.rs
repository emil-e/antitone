#![cfg(all(feature = "gpu", target_os = "macos"))]

use antitone::color::Balance;
use antitone::gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource};
use antitone::processing::{Calibration, DEFAULT_GAMMA, PhotoSettings, Pipeline, RollSettings};
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
        let mut configurations = vec![(None, ProcessingParameters::source())];
        for gamma in [0.1, DEFAULT_GAMMA, 0.65, 1.0, 3.0] {
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
                    ..PhotoSettings::default()
                };
                let parameters = ProcessingParameters::positive(cal, roll, photo, matrix).unwrap();
                configurations.push((
                    Some(Pipeline::new(cal, roll, photo, matrix).unwrap()),
                    parameters,
                ));
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
            Some(Pipeline::new(attenuated_cal, attenuated_roll, attenuated_photo, matrix).unwrap()),
            ProcessingParameters::positive(
                attenuated_cal,
                attenuated_roll,
                attenuated_photo,
                matrix,
            )
            .unwrap(),
        ));
        for contrast in [0.4, 0.5, 0.8, 1.2, 1.5, 1.6] {
            for pivot in [0.01, 0.18, 1.] {
                for colour in [0., 0.5, 1.] {
                    for saturation in [0., 0.7, 1.3, 2.] {
                        for exposure in [-16., 2., 16.] {
                            let roll = RollSettings {
                                wb: Balance {
                                    warmth: 0.14,
                                    tint: -0.18,
                                },
                                ..RollSettings::new_grade()
                            };
                            let photo = PhotoSettings {
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
                            configurations.push((
                                Some(Pipeline::new(cal, roll, photo, matrix).unwrap()),
                                ProcessingParameters::positive(cal, roll, photo, matrix).unwrap(),
                            ));
                        }
                    }
                }
            }
        }
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
                    (matrix * scan, 0)
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
fn metal_creative_primitives_cover_zero_y_continuations_and_signed_hdr_planes() {
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
                    include_str!("../src/shaders/film_inversion_precise.wgsl"),
                    include_str!("../src/shaders/processing_math.wgsl"),
                    include_str!("../src/shaders/photographic.wgsl"),
                    include_str!("../src/shaders/local_adjustments.wgsl"),
                    include_str!("../src/shaders/photographic_precise.wgsl"),
                    include_str!("../src/shaders/processing.wgsl"),
                    r#"
@group(0) @binding(5) var<storage, read> grade_high: array<vec4f>;
@group(0) @binding(6) var<storage, read> grade_low: array<vec4f>;
@group(0) @binding(7) var<storage, read_write> grade_output: array<vec4f>;
@group(0) @binding(8) var<storage, read> grade_tail: array<vec4f>;
@compute @workgroup_size(64)
fn grade_test(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&grade_high) { return; }
    if grade_high[id.x].w > 0.5 {
        let gain = ratio_gain(number(0.0), global_creative_grade());
        grade_output[id.x] = vec4f(vec3f(gain.h + (gain.l + gain.t)), 0.0);
        return;
    }
    let rgb = photographic_grade(array<Triple, 3>(
        Triple(grade_high[id.x].x, grade_low[id.x].x, grade_tail[id.x].x),
        Triple(grade_high[id.x].y, grade_low[id.x].y, grade_tail[id.x].y),
        Triple(grade_high[id.x].z, grade_low[id.x].z, grade_tail[id.x].z)));
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
        let weights = antitone::color::AP1_TO_XYZ.row(1);
        let plane = DVec3::new(0.25, -weights.x / weights.y * 0.25, 0.);
        let inputs = [
            DVec3::ZERO,
            DVec3::splat(-100.),
            DVec3::splat(0.0078125),
            DVec3::splat(0.18),
            DVec3::new(12., -0.02, 0.5),
            plane,
            plane + DVec3::Y * 1e-10,
            plane - DVec3::Y * 1e-10,
            plane + DVec3::Y * 1e-16,
            plane - DVec3::Y * 1e-16,
            plane * 4e6,
            DVec3::ZERO,
        ];
        let mut high: Vec<_> = inputs.iter().map(|v| v.as_vec3().extend(0.)).collect();
        high.last_mut().unwrap().w = 1.; // Exercise the exact analytic gain, not just a near-zero dot product.
        let low: Vec<_> = inputs
            .iter()
            .zip(&high)
            .map(|(v, h)| (v - h.truncate().as_dvec3()).as_vec3().extend(0.))
            .collect();
        let tail: Vec<_> = inputs
            .iter()
            .zip(&high)
            .zip(&low)
            .map(|((v, h), l)| {
                (v - h.truncate().as_dvec3() - l.truncate().as_dvec3())
                    .as_vec3()
                    .extend(0.)
            })
            .collect();
        let storage = |label, values: &[Vec4]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(values),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let high = storage("temporary synthetic AP1 high", &high);
        let low = storage("temporary synthetic AP1 low", &low);
        let tail = storage("temporary synthetic AP1 tail", &tail);
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
                        let photo = PhotoSettings {
                            contrast,
                            pivot,
                            contrast_colour_mix: mix,
                            saturation,
                            saturation_density: mix,
                            ..PhotoSettings::default()
                        };
                        let grade = antitone::photographic::Grade::new(photo).unwrap();
                        let parameters = ProcessingParameters::positive(
                            Calibration {
                                dmin: DVec3::splat(0.75),
                                dmax: DVec3::splat(0.01),
                                input_floor: 1e-5,
                            },
                            RollSettings::new_grade(),
                            photo,
                            DMat3::IDENTITY,
                        )
                        .unwrap();
                        let uniform =
                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("creative parameters"),
                                contents: bytemuck::bytes_of(&parameters),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                        let entries = [
                            (2, &uniform),
                            (5, &high),
                            (6, &low),
                            (7, &output),
                            (8, &tail),
                        ]
                        .map(|(binding, buffer)| wgpu::BindGroupEntry {
                            binding,
                            resource: buffer.as_entire_binding(),
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
                            let expected = if i == inputs.len() - 1 {
                                let qp = (pivot.log2() + 9.72) / 17.52;
                                let q0 = qp + contrast * (0.0729055341958355 - qp);
                                let gain = if q0 <= 0.155251141552511 {
                                    contrast
                                } else {
                                    contrast
                                        * 10.5402377416545
                                        * std::f64::consts::LN_2
                                        * 17.52
                                        * (q0 * 17.52 - 9.72).exp2()
                                };
                                DVec3::splat(gain)
                            } else {
                                grade.apply(*input).unwrap()
                            };
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
fn photographic_parameter_signatures_and_range_preflight_cover_the_new_stages() {
    let cal = Calibration {
        dmin: DVec3::splat(0.5),
        dmax: DVec3::splat(0.01),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::default();
    let photo = PhotoSettings::default();
    let base = ProcessingParameters::positive(cal, roll, photo, DMat3::IDENTITY).unwrap();
    for changed in [
        PhotoSettings {
            contrast: 1.2,
            ..photo
        },
        PhotoSettings {
            pivot: 0.25,
            ..photo
        },
        PhotoSettings {
            contrast_colour_mix: 0.25,
            ..photo
        },
        PhotoSettings {
            saturation: 1.3,
            ..photo
        },
        PhotoSettings {
            saturation_density: 0.25,
            ..photo
        },
    ] {
        let parameters =
            ProcessingParameters::positive(cal, roll, changed, DMat3::IDENTITY).unwrap();
        assert_ne!(bytemuck::bytes_of(&parameters), bytemuck::bytes_of(&base));
    }
    let extreme = RollSettings { gamma: 0.1, ..roll };
    assert!(ProcessingParameters::positive(cal, extreme, photo, DMat3::IDENTITY).is_ok());
    assert!(
        ProcessingParameters::positive(
            cal,
            extreme,
            PhotoSettings {
                contrast: 1.5,
                ..photo
            },
            DMat3::IDENTITY
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
            },
            DMat3::IDENTITY,
        )
        .is_err()
    );
    assert_eq!(std::mem::size_of::<ProcessingParameters>(), 480);
}
