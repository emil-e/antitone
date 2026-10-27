use super::*;

#[test]
fn film_emulation_gpu_gaussian_energy_and_magnification_limits() {
    pollster::block_on(async {
        let (device, queue, _, _) = context().await;
        let shader = format!(
            "{}\n{}",
            super::super::super::grain::shader(),
            r#"
@group(0) @binding(9) var<uniform> probe:vec4f;
@compute @workgroup_size(64)
fn kernel_probe(@builtin(global_invocation_id) id: vec3u) {
    let phase = vec2f(f32(id.x % 8u), f32(id.x / 8u)) / 8.0;
    if u32(probe.y) == 2u {
        let x = i32(id.x % 256u) - 128;
        let y = i32(id.x / 256u) - 128;
        positives[id.x] = vec4f(
            grain_hash(x, y, 17u), grain_hash(x + 1, y, 17u),
            grain_hash(x, y + 1, 17u), grain_hash(x, y, 77u));
        return;
    }
    if u32(probe.y) == 1u {
        let a = grain_field(vec2f(1000.0) + phase, 0.3, 0.25, 17u);
        let b = grain_field(vec2f(1000.0) + phase, 0.3, 1.0, 17u);
        let invisible = grain_field(vec2f(1000.0) + phase, 0.3, 1e5, 17u);
        positives[id.x] = vec4f(a - b, length(invisible));
        return;
    }
    let v = probe.x;
    let radius = i32(ceil(3.5 * sqrt(v) + 1.0));
    let factored_energy = grain_kernel_energy(phase, v, radius);
    var mass = 0.0; var energy = 0.0; var min_weight = 1.0;
    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let d = phase - vec2f(f32(x), f32(y));
            let q = 0.5 * dot(d, d) / v;
            let weight = exp(-q);
            mass += weight; energy += weight*weight; min_weight = min(min_weight, weight);
        }
    }
    positives[id.x] = vec4f(mass, min_weight, energy, factored_energy);
}
"#
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grain kernel and hash invariants"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("kernel_probe"),
            compilation_options: Default::default(),
            cache: None,
        });
        for (v, mode) in [
            (1. / 1.1_f32.powi(2), 0),
            (4., 0),
            (16., 0),
            (64., 0),
            (256., 0),
            (1., 1),
            (1., 2),
        ] {
            let size = if mode == 2 { [256, 256] } else { [64, 1] };
            let count: u32 = size[0] * size[1];
            let output = float_buffer(&device, size).unwrap();
            let view = uniform(&device, bytemuck::bytes_of(&[v, mode as f32, 0., 0.]));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: view.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
            }
            queue.submit([encoder.finish()]);
            let pixels = read(&device, &queue, &output);
            if mode == 2 {
                let means: [f64; 4] = std::array::from_fn(|c| {
                    pixels.iter().map(|p| p[c] as f64).sum::<f64>() / pixels.len() as f64
                });
                let variances: [f64; 4] = std::array::from_fn(|c| {
                    pixels
                        .iter()
                        .map(|p| (p[c] as f64 - means[c]).powi(2))
                        .sum::<f64>()
                        / pixels.len() as f64
                });
                assert!(means.into_iter().all(|v| v.abs() < 0.004));
                assert!(variances.into_iter().all(|v| (v * 12. - 1.).abs() < 0.03));
                assert!(pixels.iter().flatten().all(|v| v.abs() < 1.634));
                let kurtosis = pixels
                    .iter()
                    .map(|p| (p[0] as f64 - means[0]).powi(4))
                    .sum::<f64>()
                    / pixels.len() as f64
                    / variances[0].powi(2);
                assert!(
                    (2.8..3.12).contains(&kurtosis),
                    "near-Gaussian hash kurtosis {kurtosis}"
                );
                for c in 1..4 {
                    let correlation = pixels
                        .iter()
                        .map(|p| (p[0] as f64 - means[0]) * (p[c] as f64 - means[c]))
                        .sum::<f64>()
                        / pixels.len() as f64
                        / (variances[0] * variances[c]).sqrt();
                    assert!(
                        correlation.abs() < 0.025,
                        "hash neighbour/stream correlation {correlation}"
                    );
                }
                continue;
            }
            for pixel in pixels {
                if mode == 1 {
                    assert_eq!(
                        pixel, [0.; 4],
                        "magnification adds no contrast and invisible grain avoids unbounded work"
                    );
                } else {
                    let [mass, min_weight, direct_energy, factored_energy] = pixel;
                    assert!(mass > 0. && min_weight >= 0.);
                    assert!(
                        mass / direct_energy.sqrt() < 4. * v.sqrt(),
                        "kernel gain bound"
                    );
                    assert!((direct_energy / factored_energy - 1.).abs() < 0.0003);
                }
            }
        }
    });
}

#[test]
fn grain_evaluates_small_sources_and_dark_signed_pixels_without_speculative_range_rejection() {
    pollster::block_on(async {
        let (device, queue, _, kernels) = context().await;
        let settings = crate::film_emulation::FilmEmulationPreset::Medium.settings();
        let parameters = settings.parameters([64; 2]).unwrap().unwrap();
        // Small sources integrate many lattice samples; their actual finite gain is exercised.
        let raster = crate::gpu::raster::Raster::native([1000, 1000, 16, 16].into());
        let plan = crate::pixel_pipeline::PixelPipeline {
            stages: vec![crate::pixel_pipeline::PixelStage::Grain {
                parameters: parameters.grain(),
                seed: 17,
            }],
        };
        let prepared =
            crate::gpu::pixel_pipeline::Prepared::new(&device, &plan, raster, None).unwrap();
        for value in [1e-30_f32, 0.18, 2_f32.powi(80)] {
            let values = vec![[value, -value, 0., 0.]; 256];
            let pixels = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            kernels
                .stages
                .encode(&device, &mut encoder, &prepared, &pixels)
                .unwrap();
            queue.submit([encoder.finish()]);
            let result = read(&device, &queue, &pixels);
            for p in &result {
                assert!(p.iter().all(|v| v.is_finite()));
                assert!(p[0] > 0. && p[1] < 0. && p[2] == 0.);
                assert!((p[0] / value).is_finite() && (p[0] / value) > 0.);
            }
            assert!(
                result.iter().any(|p| (p[0] / value - 1.).abs() > 0.01),
                "grain must execute"
            );
        }
    });
}

#[test]
fn film_emulation_gpu_mtf_preserves_large_signed_hdr_constants() {
    pollster::block_on(async {
        let (device, queue, _, kernels) = context().await;
        let value = 2_f32.powi(122);
        let values = vec![[value, -value, 0.0001, 8.]; 65];
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("admissible large signed HDR"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = float_buffer(&device, [65, 1]).unwrap();
        for footprint in [1. / 16., 2. / 0.075, 64.] {
            let raster = crate::gpu::raster::Raster {
                dimensions: [65, 1, 8, 0],
                axis_x: [footprint, 0., 0., 0.],
                axis_y: [0., 1., 0., 0.],
                ..bytemuck::Zeroable::zeroed()
            };
            let view = crate::gpu::raster::uniform(
                &device,
                &crate::gpu::image_mtf::Parameters::new(raster, 2.),
                "MTF numerical control",
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &kernels.stages.mtf.horizontal.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: view.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: source.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&kernels.stages.mtf.horizontal);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(2, 1, 1);
            }
            queue.submit([encoder.finish()]);
            for pixel in read(&device, &queue, &output) {
                assert!(pixel.iter().all(|v| v.is_finite()));
                assert!((pixel[0] / value - 1.).abs() < 0.00003);
                assert!((pixel[1] / (-value) - 1.).abs() < 0.00003);
                assert!((pixel[2] / 0.0001 - 1.).abs() < 0.00003);
            }
        }
    });
}
