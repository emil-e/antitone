#![cfg(all(feature = "gpu", target_os = "macos"))]
use crate::photographic::graph::test_support::CameraFixture;
use crate::photographic::test_inputs::AuthoredPhotograph;
use antitone::adjustment_layers::{
    AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
};
use antitone::geometry::SourceRegion;

use crate::{
    acquisition::camera::ProcessingSource, photographic::parameters::ProcessingParameters,
};
use antitone::color::Balance;
use antitone::processing::{Calibration, InputSettings, NegativeInput, Response, RollSettings};
use glam::{DMat3, DVec3, Vec4};

use super::photographic_reference as reference;

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
    queue: &crate::gpu::submission::Queue,
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
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let fixture = CameraFixture::new(&device).await.unwrap();
        let base = DVec3::new(40000., 45000., 50000.);
        let dense = DVec3::new(5000., 7000., 9000.);
        let calibration = Calibration {
            dmin: base / 65535.,
            dmax: dense / 65535.,
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
            let source = ProcessingSource::new(&device, &codes, matrix, None).unwrap();
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
                            let denominator = base / dense - DVec3::ONE;
                            let join = base / (DVec3::ONE + 65536. * denominator);
                            let flags = (0..3)
                                .fold(0, |bits, c| bits | (u32::from(camera[c] < join[c]) << c));
                            let native = DVec3::from_array(std::array::from_fn(|c| {
                                if camera[c] <= join[c] {
                                    65536.
                                        + (65536. + 1. / denominator[c])
                                            * (1. - camera[c].max(0.) / join[c])
                                } else {
                                    (base[c] / camera[c] - 1.) / denominator[c]
                                }
                            }));
                            (native, flags)
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
fn film_base_and_signed_neighbours_follow_represented_camera_values() {
    let (device, queue) = pollster::block_on(async {
        let adapter = antitone::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap()
    });
    let fixture = pollster::block_on(CameraFixture::new(&device)).unwrap();
    let base = 0.5_f32;
    let calibration = Calibration {
        dmin: DVec3::splat(f64::from(base) + 2_f64.powi(-28)),
        dmax: DVec3::splat(0.03125),
    };
    // The f64 inputs differ but share the GPU's represented film-base value.
    // Check its zero anchor and signed neighbours rather than f64 agreement here.
    assert!(calibration.dmin.x > f64::from(base));
    assert_eq!(calibration.dmin.x as f32, base);
    let scalar = antitone::processing::InputPipeline::new(calibration, DMat3::IDENTITY).unwrap();
    assert!(
        scalar
            .apply(DVec3::splat(f64::from(base)))
            .unwrap()
            .rgb
            .min_element()
            > 0.
    );

    let pixels = [base, base.next_down(), base.next_up()]
        .map(|sample| [sample * 65535., sample * 65535., sample * 65535., 0.]);
    let source = ProcessingSource::new_continuous(&device, &pixels, DMat3::IDENTITY, None).unwrap();
    let result = capture(
        &device,
        &queue,
        &fixture,
        &source,
        ProcessingParameters::positive(calibration, RollSettings::default(), DMat3::IDENTITY)
            .unwrap(),
        pixels.len(),
    );
    assert_eq!(result[0], [0.; 4], "represented film base anchors zero");
    assert!(
        result[1][..3].iter().all(|&value| value > 0.),
        "denser represented light remains positive"
    );
    assert!(
        result[2][..3].iter().all(|&value| value < 0.),
        "above-base represented light remains signed"
    );
    assert!(result.iter().flatten().all(|value| value.is_finite()));
    assert!(result.iter().all(|pixel| pixel[3] == 0.));
}

#[test]
fn creative_operators_preserve_identity_oklab_lightness_hue_and_finite_signed_values() {
    use wgpu::util::DeviceExt;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct photographic primitive gate"),
            source: wgpu::ShaderSource::Wgsl(
                [
                    crate::photographic::stages::shader::functions(),
                    r#"
struct PrimitiveParameters { amounts: vec4f, vibrance: vec4f }
@group(0) @binding(2) var<uniform> primitive: PrimitiveParameters;
@group(0) @binding(8) var<storage, read> grade_input: array<vec4f>;
@group(0) @binding(9) var<storage, read_write> grade_output: array<vec4f>;
@compute @workgroup_size(64)
fn grade_test(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&grade_input) { return; }
    let input = grade_input[id.x];
    let rgb = colour_apply(array<f32, 3>(input.x, input.y, input.z),
        0.0, primitive.amounts.x, primitive.amounts.y, primitive.amounts.z, primitive.amounts.w, primitive.vibrance.x);
    grade_output[id.x] = finish_pixel(ProcessedPixel(rgb, 0u));
}
"#.to_owned(),
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
        let mut inputs = vec![
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
            DVec3::splat(1. / 32768.),
            DVec3::new(-1e-8, 0., 1e-8),
            DVec3::new(-1e-5, 1e-5, 0.18),
            // A near-film-base signed sample whose upstream rounding is amplified by grading.
            DVec3::new(
                -0.0005918174865655601,
                -0.00040678709046915174,
                0.00025756118702702224,
            ),
        ];
        let gradient_start = inputs.len();
        for g in [-0.18_f64, 0.001, 0.18, 4.] {
            for direction in [
                DVec3::new(1., 0., -1.),
                DVec3::new(1., -1., 0.),
                DVec3::new(0., 1., -1.),
            ] {
                for t in [-0.0005, 0., 0.0005] {
                    inputs.push(DVec3::splat(g) + g.abs() * t * direction);
                }
            }
        }
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
        let grading = crate::photographic::stages::grading::Grading::new(
            &device.limits(),
            RollSettings::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            SourceRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        )
        .unwrap();
        let context = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OKLab working-space conversion"),
            contents: grading.context_bytes(),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut contrast_effect = 0_f64;
        for contrast in [0.4_f64, 0.5, 0.8, 1., 1.2, 1.5, 1.6, 2.] {
            for pivot in [0.01, 0.03, 0.18, 1.] {
                let mut baseline_lab = vec![DVec3::ZERO; inputs.len()];
                for (mix, vibrance) in [
                    (0., 0.),
                    (0.5, 0.),
                    (1., 0.),
                    (0., -1.),
                    (0.5, 0.5),
                    (1., 1.),
                ] {
                    for saturation in [0., 1., 2.] {
                        let photo = AdjustmentLayerSettings {
                            contrast,
                            pivot,
                            contrast_colour_mix: mix,
                            saturation,
                            vibrance,
                            ..AdjustmentLayerSettings::default()
                        };
                        let prepared = antitone::photographic::Grade::new(photo).unwrap();
                        let parameters = [
                            Vec4::new(
                                (photo.contrast - 1.) as f32,
                                prepared.qp as f32,
                                photo.contrast_colour_mix as f32,
                                (photo.saturation - 1.) as f32,
                            ),
                            Vec4::new(photo.vibrance as f32, 0., 0., 0.),
                        ];
                        let uniform =
                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("creative parameters"),
                                contents: bytemuck::bytes_of(&parameters),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                        let entries = [(2, &uniform), (7, &context), (8, &input), (9, &output)]
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
                        for (i, value) in values.iter().enumerate() {
                            let input = value.truncate().as_dvec3();
                            let result = actual[i].truncate().as_dvec3();
                            assert!(result.is_finite(), "{photo:?}, input={input:?}");
                            assert_eq!(actual[i].w, 0.);
                            let contrasted =
                                reference::contrast_mix(input, contrast, pivot, mix).unwrap();
                            let expected =
                                reference::saturation(contrasted, saturation, vibrance).unwrap();
                            assert!(
                                (result - expected).abs().max_element()
                                    <= 2e-5 + 2e-4 * expected.abs().max_element(),
                                "{photo:?}: {input:?} -> {result:?}, expected {expected:?}"
                            );
                            if saturation == 1. && mix == 0. && vibrance == 0. {
                                baseline_lab[i] = reference::to_oklab(result);
                            }
                            if saturation == 1. && contrast != 1. && mix != 0. && vibrance == 0. {
                                let original_lab = reference::to_oklab(input);
                                // Compare to the GPU's own contrasted lightness, not f64 tone rounding.
                                let base_lab = baseline_lab[i];
                                let expected_lab = DVec3::new(
                                    base_lab.x,
                                    (1. - mix) * base_lab.y + mix * original_lab.y,
                                    (1. - mix) * base_lab.z + mix * original_lab.z,
                                );
                                assert!(
                                    (reference::to_oklab(result) - expected_lab)
                                        .abs()
                                        .max_element()
                                        < 2e-5,
                                    "Preserve must retain contrasted L and interpolate only a/b: {photo:?}, {input:?} -> {result:?}; actual Lab {:?}, expected {expected_lab:?}",
                                    reference::to_oklab(result)
                                );
                            }
                            if contrast == 1. {
                                let before = reference::to_oklab(input);
                                let after = reference::to_oklab(result);
                                let tolerance = 2e-5 * (1. + before.abs().max_element());
                                assert!((before.x - after.x).abs() < tolerance, "OKLab L is fixed");
                                assert!(
                                    (before.y * after.z - before.z * after.y).abs() < tolerance,
                                    "OKLab hue is fixed"
                                );
                                if saturation == 1. && vibrance == 0. {
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
                        if contrast == 1. {
                            for (samples, results) in inputs[gradient_start..]
                                .as_chunks::<3>()
                                .0
                                .iter()
                                .zip(actual[gradient_start..].as_chunks::<3>().0)
                            {
                                let h = samples[1].x.abs() * 0.0005;
                                let curvature = (results[0].truncate() + results[2].truncate()
                                    - 2. * results[1].truncate())
                                .as_dvec3()
                                    / h;
                                assert!(
                                    curvature.abs().max_element() < 0.1,
                                    "neutral-axis cusp: {photo:?}, {:?}: {curvature:?}",
                                    samples[1]
                                );
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
fn valid_creative_controls_do_not_require_a_conservative_range_proof() {
    let cal = Calibration {
        dmin: DVec3::splat(0.5),
        dmax: DVec3::splat(0.01),
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
        AuthoredPhotograph::validate_limits(
            &limits,
            extreme,
            RollSettings::default(),
            &valid,
            DMat3::from_diagonal(DVec3::splat(1e20)),
            area
        )
        .is_ok()
    );
    let extreme_grade =
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 1.5,
            ..Default::default()
        })])
        .unwrap();
    assert!(
        AuthoredPhotograph::validate_limits(
            &limits,
            extreme,
            RollSettings::default(),
            &extreme_grade,
            DMat3::IDENTITY,
            area
        )
        .is_ok()
    );
}

#[test]
fn extreme_finite_response_and_grade_controls_remain_packable_for_gpu_execution() {
    let cal = Calibration {
        dmin: DVec3::ONE,
        dmax: DVec3::splat(0.01),
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
        .and_then(|snapshot| AuthoredPhotograph::validate_limits(
            &wgpu::Limits::default(),
            interpretation,
            roll,
            &snapshot,
            DMat3::from_diagonal(DVec3::splat(1e30)),
            SourceRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        ))
        .is_ok()
    );
    assert_eq!(std::mem::size_of::<ProcessingParameters>() % 16, 0);
}
