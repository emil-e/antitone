#![cfg(all(feature = "gpu", target_os = "macos"))]
use crate::gpu::camera_fixture::CameraFixture;
use antitone::adjustment_layers::{
    AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
};
use antitone::{geometry::SourceRegion, gpu::ProcessingInput};

use antitone::color::Balance;
use antitone::gpu::{ProcessingParameters, ProcessingSource};
use antitone::processing::{Calibration, InputSettings, NegativeInput, Response, RollSettings};
use glam::{DMat3, DVec3, Vec4};

fn negative(cal: Calibration, gamma: f64, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response: Response::film(),
        gamma,
        balance,
    }
}

fn capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    fixture: &CameraFixture,
    source: &ProcessingSource,
    parameters: ProcessingParameters,
    count: usize,
) -> Vec<[f32; 4]> {
    let bytes = count as u64 * 16;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("camera primitive output"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("camera primitive readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    fixture.encode(device, &mut encoder, source, &output, parameters);
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes);
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        sender.send(result).unwrap()
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let values = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    staging.unmap();
    values
}

#[test]
fn native_negative_and_digital_primitives_preserve_analytic_values_order_and_floor_flags() {
    pollster::block_on(async {
        let adapter = antitone::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let fixture = CameraFixture::new(&device).await.unwrap();
        let base = DVec3::new(40000., 45000., 50000.);
        let dense = DVec3::new(5000., 7000., 9000.);
        let floor = 2.5;
        let calibration = Calibration {
            dmin: base / 65535.,
            dmax: dense / 65535.,
            input_floor: floor / 65535.,
        };
        let interpretation = NegativeInput {
            response: Response {
                toe: 1.,
                shoulder: 0.,
            },
            ..negative(calibration, 1., Balance::default())
        };
        let codes = [
            [40000, 45000, 50000, 0],
            [5000, 7000, 9000, 0],
            [65535, 60000, 55000, 0],
            [0, 2, 3, 0],
            [3, 1, 0, 0],
            [15000, 30000, 10000, 0],
            [30000, 10000, 15000, 0],
        ];
        let matrices = [
            DMat3::IDENTITY,
            DMat3::from_cols(
                DVec3::new(1.4, 0.1, -0.15),
                DVec3::new(-0.3, 1.1, 0.2),
                DVec3::new(0.1, -0.2, 1.3),
            ),
        ];
        let mut signed = false;
        let mut hdr = false;
        for matrix in matrices {
            let source = ProcessingSource::new(&device, &codes, matrix).unwrap();
            for input in [InputSettings::from(interpretation), InputSettings::Digital] {
                for roll in [
                    RollSettings::default(),
                    RollSettings {
                        exposure: -1.,
                        wb: Balance {
                            warmth: 0.2,
                            tint: -0.1,
                        },
                    },
                ] {
                    let parameters = ProcessingParameters::interpret(input, roll, matrix).unwrap();
                    let actual =
                        capture(&device, &queue, &fixture, &source, parameters, codes.len());
                    let transform = antitone::color::wb_matrix(roll.wb).unwrap() * matrix;
                    for (pixel, code) in actual.iter().zip(codes) {
                        let camera = DVec3::new(code[0] as f64, code[1] as f64, code[2] as f64);
                        let (native, flags) = if input.is_negative() {
                            // Gamma-one, unshaped inversion is an elementary ratio in code units.
                            let sensor = camera.max(DVec3::splat(floor));
                            let flags = (0..3)
                                .fold(0, |bits, c| bits | (u32::from(camera[c] < floor) << c));
                            (
                                (base / sensor - DVec3::ONE) / (base / dense - DVec3::ONE),
                                flags,
                            )
                        } else {
                            (camera / 65535., 0)
                        };
                        let expected = transform * native * roll.exposure.exp2();
                        signed |= expected.min_element() < 0.;
                        hdr |= expected.max_element() > 1.;
                        assert_eq!(pixel[3] as u32, flags);
                        let allowed = 2e-5 + 2e-4 * expected.abs().max_element();
                        for c in 0..3 {
                            assert!(
                                pixel[c].is_finite()
                                    && (pixel[c] as f64 - expected[c]).abs() <= allowed,
                                "{input:?} {code:?}: {pixel:?} vs {expected:?}"
                            );
                        }
                    }
                }
            }
            let inspection = capture(
                &device,
                &queue,
                &fixture,
                &source,
                ProcessingParameters::source(),
                codes.len(),
            );
            for (pixel, code) in inspection.into_iter().zip(codes) {
                let expected =
                    matrix * DVec3::new(code[0] as f64, code[1] as f64, code[2] as f64) / 65535.;
                for c in 0..3 {
                    assert!((pixel[c] as f64 - expected[c]).abs() < 1e-6);
                }
                assert_eq!(
                    pixel[3], 0.,
                    "inspection does not inherit negative floor flags"
                );
            }
        }
        assert!(
            signed && hdr,
            "fixture must detect clipping of signed/HDR results"
        );
    });
}

#[test]
fn creative_operators_preserve_identity_linear_anchors_and_finite_signed_values() {
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
                    include_str!("../../shaders/camera_conversion.wgsl"),
                    include_str!("../../shaders/flat_field.wgsl"),
                    include_str!("../../shaders/film_response.wgsl"),
                    include_str!("../../shaders/film_inversion_export.wgsl"),
                    include_str!("../../shaders/photographic.wgsl"),
                    include_str!("../../shaders/adjustment_layers.wgsl"),
                    include_str!("../../shaders/photographic_colour.wgsl"),
                    include_str!("../../shaders/photographic_export.wgsl"),
                    include_str!("../../shaders/processing.wgsl"),
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
        let y_weights = antitone::color::AP1_TO_XYZ.row(1);
        let zero_y = DVec3::new(
            0.25,
            -(y_weights.x * 0.25 - y_weights.z * 0.125) / y_weights.y,
            -0.125,
        );
        let inputs = [
            DVec3::ZERO,
            DVec3::splat(-1.),
            DVec3::splat(0.0078125),
            DVec3::splat(0.18),
            DVec3::new(4., -0.02, 0.5),
            DVec3::new(0.25, 0.5, 1.),
            DVec3::splat(0.01),
            DVec3::ONE,
            zero_y,
            zero_y + DVec3::Y * 1e-6,
            zero_y - DVec3::Y * 1e-6,
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
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("creative primitive readback"),
            size: output.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut contrast_effect = 0_f64;
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
                        let prepared = antitone::photographic::Grade::new(photo).unwrap();
                        let parameters = [
                            Vec4::new(
                                (photo.contrast - 1.) as f32,
                                prepared.qp as f32 - 0.072_905_53_f32,
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
                        for (i, value) in values.iter().enumerate() {
                            let input = value.truncate().as_dvec3();
                            let result = actual[i].truncate().as_dvec3();
                            assert!(result.is_finite(), "{photo:?}, input={input:?}");
                            assert_eq!(actual[i].w, 0.);
                            if contrast == 1. {
                                // With contrast absent, both saturation anchors are linear.
                                let grey = DVec3::splat(y_weights.dot(input));
                                let maximum = DVec3::splat(input.max_element());
                                let luminance = grey + saturation * (input - grey);
                                let density = maximum + saturation * (input - maximum);
                                let expected = luminance * (1. - mix) + density * mix;
                                assert!(
                                    (result - expected).abs().max_element()
                                        <= 2e-5 + 2e-4 * expected.abs().max_element()
                                );
                                if saturation == 1. {
                                    assert_eq!(
                                        actual[i].to_array(),
                                        value.to_array(),
                                        "identity controls preserve signed/HDR bits"
                                    );
                                }
                            }
                            if input.x == input.y && input.y == input.z {
                                assert_eq!(actual[i].x, actual[i].y);
                                assert_eq!(actual[i].y, actual[i].z);
                                if (input.x - pivot).abs() < 3e-8 {
                                    assert!(
                                        (result - input).abs().max_element() < 2e-5,
                                        "contrast fixes its neutral pivot"
                                    );
                                }
                            }
                            if contrast == 1.5 && pivot == 0.18 && saturation == 1. {
                                contrast_effect =
                                    contrast_effect.max((result - input).abs().max_element());
                            }
                        }
                        let grey_indices = [1, 0, 2, 6, 3, 7];
                        for pair in grey_indices.windows(2) {
                            assert!(
                                actual[pair[0]].x <= actual[pair[1]].x,
                                "neutral contrast remains monotone"
                            );
                        }
                        drop(data);
                        readback.unmap();
                    }
                }
            }
        }
        assert!(
            contrast_effect > 0.05,
            "the fixture must detect omitted contrast"
        );
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
        ProcessingInput::validate_limits(
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
        ProcessingInput::validate_limits(
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
        .and_then(|snapshot| ProcessingInput::validate_limits(
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
