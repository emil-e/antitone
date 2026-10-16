use super::*;

#[test]
fn ordinary_and_compensated_export_select_distinct_math_without_changing_bindings() {
    let ordinary = include_str!("shaders/processing_math.wgsl");
    let export = include_str!("shaders/export_math.wgsl");
    for lookup in [false, true] {
        let general = ProcessingKernel::shader(lookup);
        assert_eq!(ProcessingKernel::pipeline_shader(lookup, false), general);
        assert!(general.contains(ordinary));
        assert!(!general.contains(export));
        let precise = ProcessingKernel::pipeline_shader(lookup, true);
        assert!(precise.contains(export));
        assert!(!precise.contains(ordinary));
        assert!(precise.ends_with(include_str!("shaders/export.wgsl")));
        for shader in [&general, &precise] {
            for name in ["precise_log1p", "precise_exp", "precise_expm1"] {
                assert_eq!(shader.matches(&format!("fn {name}(")).count(), 1);
            }
            assert_eq!(shader.contains("fn build_response_codes("), lookup);
        }
    }
}

#[test]
fn source_inspection_is_zero_default_and_changes_only_the_source_post_matrix() {
    let base = ProcessingParameters::source();
    assert_eq!(
        bytemuck::bytes_of(&base),
        bytemuck::bytes_of(&ProcessingParameters::source_inspection(0.).unwrap())
    );
    for exposure in [-16.0_f64, -2., 2., 16.] {
        let gain = exposure.exp2();
        let adjusted = ProcessingParameters::source_inspection(exposure).unwrap();
        assert_eq!(adjusted.post, matrix_columns(DMat3::IDENTITY * gain));
        let unchanged = ProcessingParameters {
            post: base.post,
            ..adjusted
        };
        assert_eq!(bytemuck::bytes_of(&unchanged), bytemuck::bytes_of(&base));
    }
    for exposure in [f64::NAN, f64::INFINITY, -17., 17.] {
        assert!(ProcessingParameters::source_inspection(exposure).is_err());
    }
}

#[test]
fn global_output_bounds_do_not_propagate_scratch_peaks_into_locals() {
    let calibration = Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::new_grade();
    let (_, input) = ProcessingParameters::positive_with_bound(
        calibration,
        roll,
        PhotoSettings::default(),
        DMat3::IDENTITY,
    )
    .unwrap();
    for photo in [
        PhotoSettings {
            contrast: 1.05,
            ..Default::default()
        },
        PhotoSettings {
            saturation: 2.,
            ..Default::default()
        },
        PhotoSettings {
            contrast: 1.05,
            saturation: 1.2,
            ..Default::default()
        },
    ] {
        let grade = crate::photographic::Grade::new(photo).unwrap();
        let peak = grade.intermediate_bound(input);
        let (_, output, _) = grade.weighted_bounds(input, 1.);
        let (_, actual) =
            ProcessingParameters::positive_with_bound(calibration, roll, photo, DMat3::IDENTITY)
                .unwrap();
        assert_eq!(actual, output.min(peak));
        assert!(actual < peak, "{photo:?}: {actual} >= {peak}");
        if photo.contrast == 1. {
            assert_eq!(actual, 3. * input);
        } else {
            assert!(actual < peak / 2.);
        }
    }
    let matrix = DMat3::from_diagonal(DVec3::splat(f64::from(f32::MAX) / (16. * input)));
    let photo = PhotoSettings {
        contrast: 0.4,
        ..Default::default()
    };
    let bound = input * matrix.x_axis.x;
    let grade = crate::photographic::Grade::new(photo).unwrap();
    assert!(grade.weighted_bounds(bound, 1.).1 < f64::from(f32::MAX) / 4.);
    assert!(grade.intermediate_bound(bound) > f64::from(f32::MAX) / 4.);
    assert!(
        ProcessingParameters::positive(calibration, roll, photo, matrix).is_err(),
        "output fitting the range must not waive an overflowing global scratch intermediate"
    );
}

#[test]
fn source_preview_restores_green_referenced_headroom_and_keeps_inspection_relative() {
    for (wb, headroom) in [
        ([1.; 4], 1.),
        ([2., 1., 1.5, 1.], 2.),
        ([0.25, 1., 0.5, 1.], 1.),
        ([1., 2., 4., 2.], 2.),
        ([1., 1., 0.5, 3.5], 3.5),
        ([1024., 512., 768., 512.], 2.),
    ] {
        for exposure in [-16.0_f64, -2., 0., 2., 16.] {
            let adjusted = ProcessingParameters::source_preview(Some(wb), exposure).unwrap();
            assert_eq!(
                adjusted.post,
                matrix_columns(DMat3::IDENTITY * (2. * headroom * exposure.exp2()))
            );
            let base = ProcessingParameters::source();
            let unchanged = ProcessingParameters {
                post: base.post,
                post_low: base.post_low,
                ..adjusted
            };
            assert_eq!(bytemuck::bytes_of(&unchanged), bytemuck::bytes_of(&base));
        }
    }
    for exposure in [f64::NAN, f64::INFINITY, -17., 17.] {
        assert!(ProcessingParameters::source_preview(Some([1.; 4]), exposure).is_err());
    }
    // Preserve the low-level identity constructor for processing diagnostics.
    assert_eq!(
        ProcessingParameters::source().post,
        matrix_columns(DMat3::IDENTITY)
    );
}

#[test]
fn source_preview_falls_back_without_repairing_missing_or_unusable_wb() {
    for wb in [
        None,
        Some([0.; 4]),
        Some([1., 1., 1., 0.]),
        Some([1., 0., 1., 1.]),
        Some([1., f64::NAN, 1., 1.]),
        Some([f64::INFINITY, 1., 1., 1.]),
        Some([-1., 1., 1., 1.]),
        Some([f64::MAX, 1., 1., 1.]),
        Some([f64::MIN_POSITIVE; 4]),
        Some([1e-6; 4]),
        Some([1., 1e-30, 1., 1.]),
        Some([65537., 1., 1., 1.]),
    ] {
        for exposure in [-16.0_f64, 0., 16.] {
            let parameters = ProcessingParameters::source_preview(wb, exposure).unwrap();
            assert_eq!(
                parameters.post,
                matrix_columns(DMat3::IDENTITY * (2. * exposure.exp2()))
            );
        }
    }
    assert_eq!(source_preview_gain(Some([65536., 1., 1., 1.])), 131072.);
}

#[cfg(feature = "raw")]
#[test]
fn authored_source_preview_scale_agrees_with_actual_libraw_development_gains() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    for wb in [
        [1.; 4],
        [2.13, 1., 1.77, 1.],
        [0.5, 1., 0.25, 0.75],
        [1., 1., 0.5, 3.5],
    ] {
        let scan = crate::scan::Scan::open(&path, Some(wb)).unwrap();
        let before = scan
            .source()
            .camera_image()
            .unwrap()
            .pixel_codes(32, 32)
            .unwrap();
        let image = scan.source().camera_image().unwrap();
        let gain = 2. / image.pre_mul[1];
        let parameters = ProcessingParameters::source_preview(Some(scan.fixed_wb), 0.).unwrap();
        assert!((f64::from(parameters.post[0].x) - gain).abs() < 1e-6 * gain);
        assert_eq!(
            scan.source()
                .camera_image()
                .unwrap()
                .pixel_codes(32, 32)
                .unwrap(),
            before
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn source_inspection_gpu_matches_scaled_camera_to_ap1_with_zero_diagnostic_flags() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let codes = [
            [16384, 8192, 4096, 0],
            [0, 0, 0, 5432],
            [65535, 65535, 65535, 0],
            [4000, 16000, 8000, 65535],
        ];
        let matrix = DMat3::from_cols(
            DVec3::new(0.6, 0.02, -0.01),
            DVec3::new(0.4, 1.2, -0.15),
            DVec3::new(0., -0.22, 1.16),
        );
        let source = ProcessingSource::new(&device, &codes, matrix).unwrap();
        let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cases = [-16.0_f64, -2., 0., 2., 16.]
            .into_iter()
            .flat_map(|exposure| {
                [
                    (
                        ProcessingParameters::source_inspection(exposure).unwrap(),
                        exposure.exp2(),
                    ),
                    (
                        ProcessingParameters::source_preview(Some([2., 1., 1.5, 1.]), exposure)
                            .unwrap(),
                        4. * exposure.exp2(),
                    ),
                    (
                        ProcessingParameters::source_preview(Some([0.5, 1., 0.25, 1.]), exposure)
                            .unwrap(),
                        2. * exposure.exp2(),
                    ),
                ]
            });
        for (parameters, gain) in cases {
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(&device, &mut encoder, &source, &output, parameters);
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 64);
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    send.send(result).unwrap()
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range().unwrap();
            let values: &[Vec4] = bytemuck::cast_slice(&mapped);
            for (pixel, actual) in codes.iter().zip(values) {
                let camera = DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) / 65535.;
                let expected = matrix * camera * gain;
                for channel in 0..3 {
                    assert!(
                        (actual[channel] as f64 - expected[channel]).abs()
                            <= 2e-5 + 2e-4 * expected[channel].abs()
                    );
                }
                assert_eq!(actual.w, 0.);
            }
            drop(mapped);
            readback.unmap();
        }
    });
}

#[test]
fn camera_matrix_rejects_unrepresentable_values_without_repair() {
    assert_eq!(size_of::<CameraMatrix>(), 144);
    for value in [f64::NAN, f64::INFINITY, 1e300, 1e-300] {
        assert!(CameraMatrix::new(DMat3::from_diagonal(DVec3::splat(value))).is_err());
    }
    let tiny = f32::from_bits(1) as f64;
    let matrix = DMat3::from_cols(DVec3::splat(tiny), DVec3::splat(tiny), DVec3::splat(tiny));
    let individual = tiny * (30000. / 65535.);
    assert_eq!(individual as f32, 0.);
    assert!(
        (matrix * DVec3::splat(30000. / 65535.))
            .as_vec3()
            .min_element()
            > 0.
    );
    assert!(CameraMatrix::new(matrix).is_err());
    assert!(
        CameraMatrix::new(DMat3::from_diagonal(DVec3::splat(f32::MIN_POSITIVE as f64))).is_err()
    );
    assert!(CameraMatrix::new(DMat3::IDENTITY).is_ok());
}

#[cfg(target_os = "macos")]
#[test]
fn rgb16_native_normalization_matches_f64_and_source_mapping_preserves_f32_bits() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut codes: Vec<_> = (0..=65535u16).map(|v| [v, v, v, 65535]).collect();
        let mut seed = 0x619af501u32;
        for _ in 0..65536 {
            let mut pixel = [0, 0, 0, 1234];
            for v in &mut pixel[..3] {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *v = seed as u16;
            }
            codes.push(pixel);
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("conversion precision readback"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("shaders/camera_conversion.wgsl"),
                    r#"
@group(0) @binding(0) var<storage, read> scans: array<vec2u>;
@group(0) @binding(1) var<storage, read_write> output: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&scans) { return; }
    let s = convert_camera(scans[id.x]);
    output[3u*id.x] = vec4f(s.high,0.0);
    output[3u*id.x+1u] = vec4f(s.low,0.0);
    output[3u*id.x+2u] = vec4f(s.tail,0.0);
}
"#
                )
                .into(),
            ),
        });
        let conversion = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: codes.len() as u64 * 48,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: output.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for matrix in [
            DMat3::IDENTITY,
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
            DMat3::from_cols(
                DVec3::new(3., 0., -0.6),
                DVec3::new(-0.9, 3., 0.),
                DVec3::new(0., 0., 4.5),
            ),
        ] {
            let source = ProcessingSource::new(&device, &codes, matrix).unwrap();
            assert_eq!(source.byte_len(), codes.len() as u64 * 8);
            assert!(source.write(&queue, &codes[..1]).is_err());
            assert!(ProcessingSource::new(&device, &[], matrix).is_err());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &conversion.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: source.input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
            for full_precision in [true, false] {
                let mut encoder = device.create_command_encoder(&Default::default());
                let bytes = if full_precision {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&conversion);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups((codes.len() as u32).div_ceil(64), 1, 1);
                    codes.len() as u64 * 48
                } else {
                    let parameters = ProcessingParameters {
                        post: [Vec4::X, Vec4::Y, Vec4::Z],
                        ..ProcessingParameters::source()
                    };
                    kernel.encode(&device, &mut encoder, &source, &output, parameters);
                    codes.len() as u64 * 16
                };
                encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
                queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                readback
                    .slice(..bytes)
                    .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                rx.recv().unwrap().unwrap();
                let mapped = readback.slice(..bytes).get_mapped_range().unwrap();
                let data: &[Vec4] = bytemuck::cast_slice(&mapped);
                for (i, p) in codes.iter().enumerate() {
                    let camera = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.;
                    let expected = if full_precision {
                        camera
                    } else {
                        matrix * camera
                    };
                    for c in 0..3 {
                        if full_precision {
                            let actual = data[3 * i][c] as f64
                                + data[3 * i + 1][c] as f64
                                + data[3 * i + 2][c] as f64;
                            assert_eq!(
                                actual.to_bits(),
                                expected[c].to_bits(),
                                "RGB16={p:?} channel={c} matrix={matrix:?}"
                            );
                        } else {
                            assert_eq!(
                                data[i][c].to_bits(),
                                (expected[c] as f32).to_bits(),
                                "original input RGB16={p:?} channel={c} matrix={matrix:?}"
                            );
                        }
                    }
                }
                drop(mapped);
                readback.unmap();
            }
        }
    });
}
