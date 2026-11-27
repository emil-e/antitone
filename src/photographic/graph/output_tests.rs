use crate::{
    Error, evaluation::*, photographic::graph::display_output, photographic::stages::look,
};
use crate::{
    looks::{LookAsset, LookOrigin, Lut3D},
    rendering::OutputTarget,
};
use std::sync::Arc;
use wgpu::util::DeviceExt;

fn render_reference(
    renderer: &crate::render_fixture::RenderFixture,
    device: &wgpu::Device,
    source: SuppliedImage,
    target: OutputTarget,
    luts: &crate::looks::LutChain,
    destination: &wgpu::Texture,
) -> Result<crate::evaluation::recorded_image::Texture, Error> {
    let endpoint = display_output(
        source.definition().clone(),
        luts,
        &renderer.looks,
        renderer.drt.get(target),
    )?;
    crate::evaluation::recorded_image::Texture::new(
        renderer.evaluation(),
        device,
        source,
        endpoint,
        destination,
    )
}

#[cfg(all(feature = "project", feature = "ocio-reference"))]
#[test]
fn frontier_baked_luts_compose_both_targets() {
    use crate::looks::{
        LutChain,
        frontier::{Selection, Variant},
    };
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/frontier/positive-controls.json"
    ))
    .unwrap();
    let inputs = fixture["inputs"].as_array().unwrap();
    let pixels: Vec<[f32; 4]> = inputs
        .iter()
        .map(|p| {
            [
                p[0].as_f64().unwrap() as f32,
                p[1].as_f64().unwrap() as f32,
                p[2].as_f64().unwrap() as f32,
                8.,
            ]
        })
        .collect();
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let input = buffer(&device, &pixels);
    for negative in [
        None,
        crate::looks::spektrafilm::Negative::from_id("kodak_portra_400"),
    ] {
        for (variant, bank) in [(Variant::A, "A"), (Variant::B, "B")] {
            let name = format!("{}/{bank}", negative.map_or("none", |s| s.id()));
            let asset = Arc::new(
                Selection { negative, variant }
                    .prepare(&std::sync::atomic::AtomicBool::new(false))
                    .unwrap(),
            );
            let chain = LutChain {
                lut: None,
                film_and_paper: Some(asset.clone()),
            };
            for (target, key) in [
                (OutputTarget::Srgb, "Srgb"),
                (OutputTarget::DisplayP3, "DisplayP3"),
            ] {
                let output = texture(
                    &device,
                    [pixels.len() as u32, 1],
                    wgpu::TextureFormat::Rgba32Float,
                );
                let binding = render_reference(
                    &renderer,
                    &device,
                    supplied(&input, [pixels.len() as u32, 1, 0, 0]),
                    target,
                    &chain,
                    &output,
                )
                .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                binding.encode(&device, &mut encoder).unwrap();
                let submission = queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: Some(std::time::Duration::from_secs(10)),
                    })
                    .unwrap();
                let actual = read_texture(&device, &queue, &output);
                let carrier: Vec<_> = pixels
                    .iter()
                    .map(|p| {
                        asset
                            .lut()
                            .apply(glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                            .unwrap()
                    })
                    .collect();
                let expected = crate::rendering::CpuRenderer::new(target)
                    .unwrap()
                    .render(&carrier)
                    .unwrap();
                let expected: Vec<_> = expected.iter().map(|v| v.as_dvec3().to_array()).collect();
                assert_eq!(expected.len(), inputs.len());
                assert_eq!(actual.len(), inputs.len());
                let mut maximum = 0f64;
                for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                    for c in 0..3 {
                        let error = (actual[c] as f64 - expected[c]).abs();
                        maximum = maximum.max(error);
                        assert!(
                            error < 2e-5,
                            "{name}/{key} control {i} channel {c}: {actual:?} {expected:?}, error {error}"
                        );
                    }
                }
                eprintln!(
                    "{name}/{key}: {} controls, max linear RGB error {maximum}",
                    actual.len()
                );
            }
            let a = renderer.looks.resource(asset.lut()).unwrap();
            let b = renderer.looks.resource(asset.lut()).unwrap();
            assert!(Arc::ptr_eq(&a, &b));
        }
    }
}
fn buffer(device: &wgpu::Device, pixels: &[[f32; 4]]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(pixels),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    })
}

fn supplied(source: &wgpu::Buffer, dimensions: [u32; 4]) -> SuppliedImage {
    let domain = Domain {
        bounds: Some(CellRegion::new([0, 0], [dimensions[0], dimensions[1]])),
        grid: Grid {
            coverage: dimensions[2],
            ..Grid::axis_aligned([0.; 2], [1.; 2])
        },
    };
    SuppliedImage::from_buffer(
        borrowed_input(domain),
        domain.finite().unwrap(),
        source.clone().into(),
    )
    .unwrap()
}

fn texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn asset() -> Arc<LookAsset> {
    let mut nodes = vec![];
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                nodes.push([(r * g) as f32, (g * b) as f32, (b * r) as f32]);
            }
        }
    }
    Arc::new(
        LookAsset::new(
            "Tetrahedral hinges".into(),
            LookOrigin::Renderer {
                renderer: "unregistered_provider".into(),
                negative: None,
                revision: "a".repeat(64),
                generator: "authored-hinges".into(),
                attribution: "Synthetic nonseparable corner products".into(),
            },
            Arc::new(Lut3D::new(2, [-0.5; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    )
}

fn read_texture(
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    let row = (texture.width() * 16).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |r| {
        send.send(r).unwrap();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let mapped = readback.get_mapped_range(..).unwrap();
    let values = mapped
        .chunks_exact(row as usize)
        .flat_map(|line| {
            bytemuck::cast_slice::<u8, [f32; 4]>(&line[..texture.width() as usize * 16])
                .iter()
                .copied()
        })
        .collect();
    drop(mapped);
    readback.unmap();
    values
}

// Explicit pre-composition sequence is an f32/lifetime regression control, not an independent
// photographic oracle. OCIO below independently checks the output transform and nonlinear LUT.
fn legacy(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    renderer: &crate::render_fixture::RenderFixture,
    source: &wgpu::Buffer,
    (dimensions, target, look): ([u32; 4], OutputTarget, Option<&Arc<LookAsset>>),
    output: &wgpu::Texture,
) {
    let looked = look.map(|asset| {
        let lut = renderer.looks.resource(asset.lut()).unwrap();
        let kernel =
            pollster::block_on(crate::photographic::stages::look::Kernel::new(device)).unwrap();
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: source.size(),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let dispatch = kernel
            .bind(
                device,
                source,
                &pixels,
                lut,
                dimensions[0] * dimensions[1],
                dimensions[2] != 0,
            )
            .unwrap();
        kernel.encode(encoder, &dispatch);
        pixels
    });
    let dimensions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&dimensions),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let resources = renderer.drt.get(target);
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &resources.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: looked.as_ref().unwrap_or(source).as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&resources.tables[0]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&resources.tables[1]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&resources.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: dimensions.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: resources.hues.as_entire_binding(),
            },
        ],
    });
    let view = output.create_view(&Default::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(&resources.pipeline);
    pass.set_bind_group(0, &group, &[]);
    pass.draw(0..3, 0..1);
}

#[test]
fn reference_composition_preserves_target_coverage_and_legacy_arithmetic_without_a_delivery_copy() {
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let pixels = [
        [-0.04, 0.18, 2., 8.],
        [0., 0., 0., 8.],
        [1.5, 4., 0.1, 9.],
        [0.2, 0.8, 0.1, 0.],
        [0.8, 0.03, 0.4, 15.],
        [0.18, 0.18, 0.18, 8.],
    ];
    let source = buffer(&device, &pixels);
    let look = asset();
    let mut captures = vec![];
    let programs = renderer.evaluation.programs.len();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        for coverage in [0, 1] {
            for selected in [None, Some(&look)] {
                let output = texture(&device, [3, 2], wgpu::TextureFormat::Rgba32Float);
                let expected = texture(&device, [3, 2], wgpu::TextureFormat::Rgba32Float);
                let dimensions = [3, 2, coverage, 0];
                let binding = render_reference(
                    &renderer,
                    &device,
                    supplied(&source, dimensions),
                    target,
                    &(selected).cloned().into(),
                    &output,
                )
                .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                binding.encode(&device, &mut encoder).unwrap();
                drop(binding);
                legacy(
                    &device,
                    &mut encoder,
                    &renderer,
                    &source,
                    (dimensions, target, selected),
                    &expected,
                );
                let submission = queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: Some(std::time::Duration::from_secs(10)),
                    })
                    .unwrap();
                let actual = read_texture(&device, &queue, &output);
                assert_eq!(actual, read_texture(&device, &queue, &expected));
                for (i, pixel) in actual.iter().enumerate() {
                    if coverage != 0 && i == 3 {
                        assert_eq!(*pixel, [0.; 4]);
                    } else {
                        assert_eq!(pixel[3], 1.);
                    }
                }
                #[cfg(feature = "ocio-reference")]
                {
                    use glam::DVec3;
                    let scene = pixels
                        .iter()
                        .map(|p| {
                            let v = DVec3::from_array([p[0] as f64, p[1] as f64, p[2] as f64]);
                            match selected {
                                Some(asset) => asset.lut().apply(v).unwrap(),
                                None => v,
                            }
                        })
                        .collect::<Vec<_>>();
                    let independent = crate::rendering::CpuRenderer::new(target)
                        .unwrap()
                        .render(&scene)
                        .unwrap();
                    for (i, (actual, expected)) in actual.iter().zip(independent).enumerate() {
                        if coverage != 0 && i == 3 {
                            continue;
                        }
                        for c in 0..3 {
                            assert!(
                                (actual[c] - expected[c]).abs() < 2e-5,
                                "{target:?} coverage={coverage} looked={} pixel={i} channel={c}: {actual:?} {expected:?}",
                                selected.is_some()
                            );
                        }
                    }
                }
                let domain = Domain {
                    bounds: Some([0, 0, 3, 2].into()),
                    grid: Grid {
                        coverage,
                        ..Grid::axis_aligned([0.; 2], [1.; 2])
                    },
                };
                let image = display_output(
                    borrowed_input(domain),
                    &(selected).cloned().into(),
                    &renderer.looks,
                    renderer.drt.get(target),
                )
                .unwrap();
                let plan = Job::new(image, domain.finite().unwrap(), Tiling { tile: [3, 2] })
                    .unwrap()
                    .texture_plan(&output)
                    .unwrap();
                let fragment = &plan.tiles[0].fragment;
                assert!(
                    !fragment
                        .resources
                        .iter()
                        .any(|r| matches!(r, Storage::Texture(_))),
                    "delivery must use the caller's attachment"
                );
                assert_eq!(
                    fragment.work.len(),
                    if selected.is_some() { 2 } else { 1 },
                    "look and target render require no delivery pass"
                );
                captures.push(actual);
            }
        }
    }
    assert!(
        captures[0]
            .iter()
            .zip(&captures[1])
            .any(|(a, b)| (0..3).any(|c| (a[c] - b[c]).abs() > 1e-3)),
        "fixture must expose omitted look"
    );
    assert!(
        captures[0]
            .iter()
            .zip(&captures[4])
            .any(|(a, b)| (0..3).any(|c| (a[c] - b[c]).abs() > 1e-3)),
        "fixture must expose wrong target"
    );
    assert_eq!(read_buffer(&device, &queue, &source), pixels);
    assert_eq!(
        renderer.evaluation.programs.len(),
        programs,
        "presentation must reuse initialization's compiled work"
    );
}

#[cfg(feature = "ocio-reference")]
#[test]
fn rec2020_presets_preserve_the_declared_peak_and_match_pinned_ocio() {
    use crate::rendering::{CpuRenderer, reference_parameters};
    use glam::DVec3;

    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let levels = [-0.2, 0., 0.002, 0.18, 1., 8., 32., 1000.];
    let size = [32, 16];
    let mut samples: Vec<_> = (0..size[0] * size[1])
        .map(|i| {
            let i = i as usize;
            [levels[i % 8], levels[(i / 8) % 8], levels[i / 64], 8.]
        })
        .collect();
    samples[0] = [1000., 1000., 1000., 8.];
    samples[1] = [8., 8., 8., 8.];
    let working: Vec<_> = samples
        .iter()
        .map(|rgb| DVec3::new(rgb[0].into(), rgb[1].into(), rgb[2].into()))
        .collect();
    let input = buffer(&device, &samples);
    let mut highlight = 0.;
    for (target, peak) in OutputTarget::HDR.into_iter().zip([5., 10., 20., 40.]) {
        assert_eq!(target.parameters(), reference_parameters(target));
        let output = texture(&device, size, wgpu::TextureFormat::Rgba32Float);
        let binding = render_reference(
            &renderer,
            &device,
            supplied(&input, [size[0], size[1], 0, 0]),
            target,
            &Default::default(),
            &output,
        )
        .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        binding.encode(&device, &mut encoder).unwrap();
        queue.submit([encoder.finish()]);
        let actual = read_texture(&device, &queue, &output);
        let cpu = CpuRenderer::new(target).unwrap().render(&working).unwrap();
        assert_eq!(actual.len(), cpu.len());
        for (rgba, reference) in actual.iter().zip(cpu) {
            for (&channel, expected) in rgba.iter().zip(reference.to_array()) {
                assert!(
                    (channel - expected).abs() <= 2e-5 + 2e-4 * expected.abs(),
                    "{target:?}: {channel} vs {expected}",
                );
            }
        }
        assert_eq!(actual[0][..3], [peak; 3]);
        // A fixed input well below the ceiling must change with the ACES tone scale.
        let value = actual[1][0];
        assert!(value > highlight && value < peak, "{target:?}: {value}");
        highlight = value;
    }
}

#[test]
fn resident_recording_reads_updated_input_and_survives_owner_retirement() {
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let first = [[0.18f32, 0.2, 0.3, 8.]];
    let next = [[0.75f32, 0.9, 1.1, 8.]];
    let input = buffer(&device, &first);
    let control = buffer(&device, &next);
    let initial = texture(&device, [1, 1], wgpu::TextureFormat::Rgba32Float);
    let expected = texture(&device, [1, 1], wgpu::TextureFormat::Rgba32Float);
    let luts = Some(asset()).into();
    let record = |source, output| {
        render_reference(
            &renderer,
            &device,
            supplied(source, [1, 1, 8, 0]),
            OutputTarget::Srgb,
            &luts,
            output,
        )
        .unwrap()
    };
    let binding = record(&input, &initial);
    let control = record(&control, &expected);
    let mut encoder = device.create_command_encoder(&Default::default());
    binding.encode(&device, &mut encoder).unwrap();
    control.encode(&device, &mut encoder).unwrap();
    queue.submit([encoder.finish()]);
    let before = read_texture(&device, &queue, &initial);
    let expected = read_texture(&device, &queue, &expected);
    assert_ne!(
        before, expected,
        "the two input samples must render differently"
    );

    queue.write_buffer(&input, 0, bytemuck::cast_slice(&next));
    let mut encoder = device.create_command_encoder(&Default::default());
    binding.encode(&device, &mut encoder).unwrap();
    let commands = encoder.finish();
    drop((binding, control, renderer, input, luts));
    queue.submit([commands]);
    assert_eq!(read_texture(&device, &queue, &initial), expected);
}

#[test]
fn display_and_look_values_preserve_sample_flags_and_ignore_uncovered_nonfinite_light() {
    for look in look_assets() {
        compare_sample_flags(look);
    }
}

fn look_assets() -> Vec<Arc<LookAsset>> {
    let assets = vec![asset()];
    #[cfg(feature = "project")]
    let assets = {
        use crate::looks::frontier::{Selection, Variant};
        let mut assets = assets;
        for variant in Variant::all() {
            assets.push(Arc::new(
                Selection {
                    negative: None,
                    variant,
                }
                .prepare(&std::sync::atomic::AtomicBool::new(false))
                .unwrap(),
            ));
        }
        assets
    };
    assets
}

fn compare_sample_flags(look: Arc<LookAsset>) {
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let source = buffer(
        &device,
        &[
            [-0.04, 0.18, 2., 9.],
            [0., 0., 0., 8.],
            [f32::NAN, 0.2, 0.3, 0.],
            [0.2, f32::NAN, 0.3, 8.],
            [0.2, 0.3, 0.4, -1.],
        ],
    );
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        for policy in [Policy::Materialized, Policy::Composed] {
            for selected in [None, Some(&look)] {
                let argument = supplied(&source, [5, 1, 8, 0]);
                let domain = argument.definition().domain();
                let image = display_output(
                    argument.definition().clone(),
                    &(selected).cloned().into(),
                    &renderer.looks,
                    renderer.drt.get(target),
                )
                .unwrap();
                let context = renderer.evaluation();
                let request = context
                    .request(
                        image,
                        Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        Tiling { tile: [5, 1] },
                    )
                    .unwrap()
                    .with_policy(policy);
                let recorded = context
                    .record(request, context.materializations(), vec![argument])
                    .unwrap();
                let output = buffer(&device, &[[0.; 4]; 5]);
                let mut encoder = device.create_command_encoder(&Default::default());
                recorded
                    .encode_result(&device, &mut encoder, &output)
                    .unwrap();
                queue.submit([encoder.finish()]);
                let actual = read_buffer(&device, &queue, &output);
                assert_eq!(
                    actual.iter().map(|p| p[3]).collect::<Vec<_>>(),
                    [9., 8., 0., -1., -1.]
                );
                assert_eq!(
                    actual[2], [0.; 4],
                    "uncovered light is not a failed contribution"
                );
                assert!(actual[0][..3].iter().all(|v| v.is_finite()));
                assert!(actual[0][..3].iter().any(|v| *v > 0.));
            }
        }
    }
}

#[test]
fn named_look_values_keep_logical_identity_and_reuse_cached_pixels() {
    struct Pixel(Domain);
    impl Source for Pixel {
        fn domain(&self) -> Domain {
            self.0
        }
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new([0.18f32, 0.2, 0.3, 8.].map(f32::to_bits)))
        }
        fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            assert_eq!(region, self.0.finite().unwrap());
            Ok(vec![[0.18, 0.2, 0.3, 8.]])
        }
    }
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let other =
        pollster::block_on(crate::render_fixture::RenderFixture::new(&device, &queue)).unwrap();
    let domain = Domain {
        bounds: Some([0, 0, 1, 1].into()),
        grid: Grid {
            coverage: 8,
            ..Grid::axis_aligned([0.; 2], [1.; 2])
        },
    };
    #[derive(Debug, PartialEq, Hash)]
    struct Nonfinite;
    impl Operation for Nonfinite {
        fn key(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new(Self))
        }
        fn reads(&self, region: CellRegion) -> Result<Vec<crate::evaluation::Read>, Error> {
            Ok(vec![crate::evaluation::Read::new(0, region)])
        }
        fn calculation(
            self: std::sync::Arc<Self>,
            inputs: &[crate::evaluation::value::Value],
            domain: Domain,
            output: CellRegion,
            _budget: crate::evaluation::Tiling,
        ) -> Result<Option<crate::evaluation::value::Calculation>, Error> {
            let inputs = crate::evaluation::value::Calculation::references(inputs);
            let function = Function::new(
                "fn nonfinite(p: vec4f, xy: vec2f) -> vec4f { return vec4f(bitcast<f32>(0x7fc00000u), p.gba); }",
                "nonfinite",
            );
            value::Calculation::new(
                domain,
                output,
                vec![inputs[0].request(output)?],
                value::Operation::shader_function(self.key(), function),
            )
            .map(Some)
        }
    }
    let base = source(Arc::new(Pixel(domain)));
    let different = Arc::new(
        LookAsset::new(
            "Different numeric look".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [-0.5; 3], [1.; 3], vec![[0.3; 3]; 8]).unwrap()),
        )
        .unwrap(),
    );
    let context = renderer.evaluation();
    let scope = context.materializations();
    let budget = Tiling { tile: [1, 1] };
    for (fails, paired) in [(false, false), (true, false), (false, true), (true, true)] {
        let chain = |first: &Arc<LookAsset>| crate::looks::LutChain {
            lut: Some(first.clone()),
            film_and_paper: paired.then(|| different.clone()),
        };
        let look = asset();
        let input = if fails {
            Value::from_operation(domain, Some(base.clone()), Nonfinite)
        } else {
            base.clone()
        };
        let pixels = buffer(
            &device,
            &[[if fails { f32::NAN } else { 0.18 }, 0.2, 0.3, 8.]],
        );
        let argument =
            SuppliedImage::from_buffer(input.clone(), domain.finite().unwrap(), pixels.into())
                .unwrap();
        let selected = chain(&look);
        let display = renderer.drt.get(OutputTarget::Srgb);
        let renamed = Arc::new(
            LookAsset::new(
                "Different label".into(),
                LookOrigin::Imported,
                look.lut().clone(),
            )
            .unwrap(),
        );
        let looked = renderer.looks.apply(input.clone(), &selected).unwrap();
        let independent = other.looks.apply(input.clone(), &chain(&renamed)).unwrap();
        let resources = look::applied_luts(&looked);
        assert_eq!(
            crate::looks::LutChain::ordered_id(resources.iter().map(|lut| lut.id())),
            selected.id()
        );
        assert_eq!(resources.len(), if paired { 2 } else { 1 });
        if paired {
            let reversed = renderer
                .looks
                .apply(
                    input.clone(),
                    &crate::looks::LutChain {
                        lut: Some(different.clone()),
                        film_and_paper: Some(look.clone()),
                    },
                )
                .unwrap();
            assert!(
                !looked.same_definition(&reversed),
                "ordered LUT operands must not alias a reversed calculation"
            );
        }

        let changed = renderer
            .looks
            .apply(input.clone(), &chain(&different))
            .unwrap();
        assert!(!looked.same_definition(&changed));
        assert!(looked.identity().is_some());
        assert!(
            looked.same_definition(&independent),
            "allocations and labels are not mathematical content"
        );
        let displayed = display.apply(looked.clone()).unwrap();
        assert!(displayed.identity().is_some());
        let other_display = other
            .drt
            .get(OutputTarget::Srgb)
            .apply(independent)
            .unwrap();
        assert!(displayed.same_definition(&other_display));
        let p3 = renderer
            .drt
            .get(OutputTarget::DisplayP3)
            .apply(looked.clone())
            .unwrap();
        assert!(!displayed.same_definition(&p3));
        let evaluation = context
            .request(
                looked,
                Region {
                    bounds: domain.finite().unwrap(),
                    grid: domain.grid,
                },
                budget,
            )
            .unwrap();
        let cached = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &scope,
                    evaluation,
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        let output = texture(&device, [1, 1], wgpu::TextureFormat::Rgba32Float);
        let readout = display_output(input.clone(), &selected, &renderer.looks, display).unwrap();
        assert!(
            readout.same_definition(&displayed),
            "a physical attachment is separate from the reusable displayed calculation"
        );
        let request = context
            .request(
                readout,
                Region {
                    bounds: domain.finite().unwrap(),
                    grid: domain.grid,
                },
                budget,
            )
            .unwrap();
        let recorded = context
            .record_to_texture(request, scope.clone(), vec![argument.clone()], &output)
            .unwrap();
        assert!(
            recorded
                .image_inputs()
                .any(|image| Arc::ptr_eq(image, &cached.image)),
            "control: readout must actually consume the stored look value"
        );
        let binding = render_reference(
            &renderer,
            &device,
            argument.clone(),
            OutputTarget::Srgb,
            &chain(&look),
            &output,
        )
        .unwrap();
        for _ in 0..2 {
            let mut encoder = device.create_command_encoder(&Default::default());
            binding.encode(&device, &mut encoder).unwrap();
            queue.submit([encoder.finish()]);
        }
        // The display transform still handles nonfinite inputs without a Look-specific check.
        assert_eq!(read_texture(&device, &queue, &output)[0][3] < 0., fails);
    }
}

#[test]
fn float_attachment_preserves_scene_linear_rgb_and_converts_coverage_to_alpha() {
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let pixels = [[-0.125, 0.25, 3.5, 8.], [1.5, -0.25, 0.5, 9.]];
    let source = supplied(&buffer(&device, &pixels), [2, 1, 8, 0]);
    let output = texture(&device, [2, 1], wgpu::TextureFormat::Rgba32Float);
    let recording = crate::evaluation::recorded_image::Texture::new(
        renderer.evaluation(),
        &device,
        source.clone(),
        source.definition().clone(),
        &output,
    )
    .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    recording.encode(&device, &mut encoder).unwrap();
    queue.submit([encoder.finish()]);
    assert_eq!(
        read_texture(&device, &queue, &output),
        [[-0.125, 0.25, 3.5, 1.], [1.5, -0.25, 0.5, 1.]]
    );
}

#[test]
fn display_attachment_admission_checks_format_extent_and_complete_grid() {
    let (device, _queue, renderer) = crate::photographic::test_support::render_context();
    let domain = Domain {
        bounds: Some([0, 0, 3, 2].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    };
    let budget = Tiling { tile: [3, 2] };
    let image = renderer
        .drt
        .get(OutputTarget::Srgb)
        .apply(borrowed_input(domain))
        .unwrap();
    let wrong_size = texture(&device, [2, 2], wgpu::TextureFormat::Rgba32Float);
    assert!(
        Job::new(image.clone(), domain.finite().unwrap(), budget)
            .unwrap()
            .texture_plan(&wrong_size)
            .is_err()
    );
    let wrong_format = texture(&device, [3, 2], wgpu::TextureFormat::Rgba16Float);
    assert!(
        Job::new(image.clone(), domain.finite().unwrap(), budget)
            .unwrap()
            .texture_plan(&wrong_format)
            .is_err()
    );
    let output = texture(&device, [3, 2], wgpu::TextureFormat::Rgba32Float);
    let evaluation = renderer
        .evaluation
        .request(
            image.clone(),
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            budget,
        )
        .unwrap();
    let recorded = renderer
        .evaluation
        .record_to_texture(
            evaluation,
            renderer.evaluation.test_materializations(true),
            Vec::new(),
            &output,
        )
        .unwrap();
    let pixels = buffer(&device, &[[0.; 4]; 6]);
    let mut encoder = device.create_command_encoder(&Default::default());
    let error = recorded
        .encode_into(&device, &mut encoder, &pixels, &pixels)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        Error::InvalidParameter("buffer sink requires raster storage").to_string()
    );
    assert!(
        Job::new(image.clone(), [0, 0, 2, 1].into(), budget)
            .unwrap()
            .texture_plan(&output)
            .is_err()
    );
    assert!(
        Job::new(image, domain.finite().unwrap(), Tiling { tile: [2, 1] })
            .unwrap()
            .texture_plan(&output)
            .is_err()
    );
}

#[cfg(feature = "ocio-reference")]
#[test]
fn two_lut_operations_apply_in_declared_order_with_independent_bypass_and_shared_resources() {
    use crate::looks::LutChain;
    use glam::DVec3;
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
    let make = |swap: bool| {
        let mut nodes = Vec::new();
        for b in [-0.5, 1.] {
            for g in [-0.5, 1.] {
                for r in [-0.5, 1.] {
                    nodes.push(if swap { [g, r, b] } else { [0.5 * r, g, b] });
                }
            }
        }
        Arc::new(
            LookAsset::new(
                if swap { "Swap" } else { "Half red" }.into(),
                LookOrigin::Imported,
                Arc::new(Lut3D::new(2, [-0.5; 3], [1.; 3], nodes).unwrap()),
            )
            .unwrap(),
        )
    };
    let swap = make(true);
    let half = make(false);
    let encoded = [DVec3::new(-0.2, 0.4, 0.8), DVec3::new(0.6, 0.3, -0.1)];
    let pixels: Vec<_> = encoded
        .iter()
        .map(|v| {
            let rgb = v.map(look::decode);
            [rgb.x as f32, rgb.y as f32, rgb.z as f32, 8.]
        })
        .collect();
    let input = buffer(&device, &pixels);
    let cpu = crate::rendering::CpuRenderer::new(OutputTarget::Srgb).unwrap();
    for (first, second, reversed) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (true, true, true),
    ] {
        let chain = if reversed {
            LutChain {
                lut: Some(half.clone()),
                film_and_paper: Some(swap.clone()),
            }
        } else {
            LutChain {
                lut: first.then(|| swap.clone()),
                film_and_paper: second.then(|| half.clone()),
            }
        };
        let expected: Vec<_> = encoded
            .iter()
            .map(|v| {
                let out = if reversed {
                    DVec3::new(v.y, v.x * 0.5, v.z)
                } else {
                    let v = if first { DVec3::new(v.y, v.x, v.z) } else { *v };
                    if second {
                        DVec3::new(v.x * 0.5, v.y, v.z)
                    } else {
                        v
                    }
                };
                out.map(look::decode)
            })
            .collect();
        let expected = cpu.render(&expected).unwrap();
        let output = texture(&device, [2, 1], wgpu::TextureFormat::Rgba32Float);
        let binding = render_reference(
            &renderer,
            &device,
            supplied(&input, [2, 1, 0, 0]),
            OutputTarget::Srgb,
            &chain,
            &output,
        )
        .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        binding.encode(&device, &mut encoder).unwrap();
        let submission = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        let actual = read_texture(&device, &queue, &output);
        for (actual, expected) in actual.iter().zip(expected) {
            for c in 0..3 {
                assert!(
                    (actual[c] - expected[c]).abs() < 2e-5,
                    "first={first} second={second} reversed={reversed}: {actual:?} {expected:?}"
                );
            }
        }
        if first && second {
            let a = renderer.looks.resource(swap.lut()).unwrap();
            let b = renderer.looks.resource(swap.lut()).unwrap();
            assert!(Arc::ptr_eq(&a, &b));
            assert_ne!(chain.id(), Some(swap.id()));
            let reverse = LutChain {
                lut: chain.film_and_paper.clone(),
                film_and_paper: chain.lut.clone(),
            };
            assert_ne!(chain.id(), reverse.id());
        }
    }
}
