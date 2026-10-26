use super::*;

#[test]
fn film_emulation_gpu_gaussian_energy_and_magnification_limits() {
    pollster::block_on(async {
        let (device, queue, _, _) = context().await;
        let shader = format!(
            "{}\n{}",
            film_shader(),
            r#"
@compute @workgroup_size(64)
fn kernel_probe(@builtin(global_invocation_id) id: vec3u) {
    let phase = vec2f(f32(id.x % 8u), f32(id.x / 8u)) / 8.0;
    if view.dimensions.w == 2u {
        let x = i32(id.x % 256u) - 128;
        let y = i32(id.x / 256u) - 128;
        positives[id.x] = vec4f(
            grain_hash(x, y, 17u), grain_hash(x + 1, y, 17u),
            grain_hash(x, y + 1, 17u), grain_hash(x, y, 77u));
        return;
    }
    if view.dimensions.w == 1u {
        let a = grain_field(vec2f(1000.0) + phase, 0.3, 0.25, 17u);
        let b = grain_field(vec2f(1000.0) + phase, 0.3, 1.0, 17u);
        let invisible = grain_field(vec2f(1000.0) + phase, 0.3, 1e5, 17u);
        positives[id.x] = vec4f(a - b, length(invisible));
        return;
    }
    let v = view.axis_x.x;
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
            let view = ViewUniform {
                dimensions: [size[0], size[1], 0, mode],
                dx: [v, 0., 0., 0.],
                ..bytemuck::Zeroable::zeroed()
            };
            let view = uniform(&device, bytemuck::bytes_of(&view));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
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
fn film_emulation_dark_input_cannot_admit_an_unrepresentable_gain() {
    let parameters = crate::film_emulation::FilmEmulationPreset::Medium
        .settings()
        .parameters(64)
        .unwrap()
        .unwrap();
    assert!(
        parameters.max_log_gain(1.) > f64::from(f32::MAX).ln(),
        "unsafe exponent control"
    );
    assert!(
        (1e-30_f64).ln() + parameters.max_log_gain(1.) < f64::from(f32::MAX).ln(),
        "product-only guard would admit"
    );
    assert!(parameters.require_output_bound(1e-30, 1.).is_err());
}

#[test]
fn film_emulation_rejects_an_unsafe_photographic_gain_before_dispatch() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let base = probe_view(&device, &queue, &renderer, &kernels, [2., 1.], false, 17);
        let old = &base.graded.as_ref().unwrap().source;
        let matrix = DMat3::IDENTITY * 1e28;
        let source = Arc::new(ViewSource {
            spec: ViewSourceSpec::new(17, old.spec.frame, InputSettings::Digital, matrix, None)
                .unwrap(),
            pixels: old.pixels.clone(),
            renderer,
            kernels,
        });
        let snapshot = AdjustmentLayerSnapshot::new(vec![]).unwrap();
        let grade = Arc::new(
            GpuAdjustmentLayers::new(
                &device,
                InputSettings::Digital,
                RollSettings::default(),
                snapshot.clone(),
                matrix,
                source.spec.area,
            )
            .unwrap(),
        );
        for amount in [0., 0.2] {
            let input = ProcessingInput::from(grade.clone())
                .with_film_emulation(FilmEmulationSettings {
                    enabled: true,
                    size: 2. / 6000. * 100.,
                    amount,
                    colour: 1.,
                })
                .with_film_seed(17);
            let result = RenderedView::new(
                &device,
                source.clone(),
                InputSettings::Digital,
                RollSettings::default(),
                snapshot.clone(),
                &input,
                OutputTarget::Srgb,
                None,
            );
            if amount == 0. {
                assert!(result.is_ok());
            } else {
                assert!(
                    matches!(result, Err(Error::Gpu(message)) if message.contains("film emulation"))
                );
            }
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
            let view = ViewUniform {
                dimensions: [65, 1, 0, 0],
                dx: [footprint, 0., 0., 0.],
                film: [1., 0., 2., 0.],
                ..bytemuck::Zeroable::zeroed()
            };
            let view = uniform(&device, bytemuck::bytes_of(&view));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &kernels.film.horizontal.get_bind_group_layout(0),
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
                pass.set_pipeline(&kernels.film.horizontal);
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
