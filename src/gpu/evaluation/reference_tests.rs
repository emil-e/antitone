use super::*;
use crate::{
    looks::{LookAsset, LookOrigin, Lut3D},
    rendering::{OutputReference, OutputTarget},
};
use wgpu::util::DeviceExt;

fn buffer(device: &wgpu::Device, pixels: &[[f32; 4]]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(pixels),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    })
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
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [-0.5; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    )
}

fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
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
    renderer: &crate::renderer::GpuRenderer,
    source: &wgpu::Buffer,
    (dimensions, target, look): ([u32; 4], OutputTarget, Option<&Arc<LookAsset>>),
    output: &wgpu::Texture,
) {
    let looked = look.map(|asset| {
        let lut = renderer.look_resource(device, asset).unwrap();
        let kernel = pollster::block_on(crate::gpu::look::Kernel::new(device)).unwrap();
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
    let resources = renderer.target(target);
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
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
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
                let binding = renderer
                    .render_binding(&device, &source, dimensions, target, selected, &output)
                    .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                let check = binding.encode(&device, &mut encoder).unwrap();
                assert_eq!(check.is_some(), selected.is_some());
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
                if let Some(check) = &check {
                    check.begin(|| {});
                }
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: Some(std::time::Duration::from_secs(10)),
                    })
                    .unwrap();
                if let Some(check) = check {
                    check.result().unwrap().unwrap();
                }
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
                    bounds: [0, 0, 3, 2].into(),
                    grid: Grid {
                        coverage,
                        ..Grid::axis_aligned([0.; 2], [1.; 2])
                    },
                    meaning: Meaning::Graded,
                };
                let reference = Reference::capture(
                    &device,
                    &renderer,
                    OutputReference::DisplayReferred(target),
                    selected,
                )
                .unwrap();
                let image = reference
                    .presentation(borrowed_input(domain), &renderer, &output)
                    .unwrap();
                let plan = Plan::new(
                    image,
                    domain.bounds,
                    Budget {
                        tile: [3, 2],
                        scratch_bytes: 1 << 20,
                    },
                )
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
    assert_eq!(execute::read(&device, &queue, &source), pixels);
    assert_eq!(
        renderer.evaluation.programs.len(),
        programs,
        "presentation must reuse initialization's compiled work"
    );
}

#[test]
fn repeated_presentation_owns_fresh_admission_and_rejects_only_covered_nonfinite_samples() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let source = buffer(
        &device,
        &[[f32::NAN, 0.2, 0.3, 0.], [0.2, f32::INFINITY, 0.3, 8.]],
    );
    let output = texture(&device, [2, 1], wgpu::TextureFormat::Rgba32Float);
    let look = asset();
    let binding = renderer
        .render_binding(
            &device,
            &source,
            [2, 1, 1, 0],
            OutputTarget::Srgb,
            Some(&look),
            &output,
        )
        .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    let bad = binding.encode(&device, &mut encoder).unwrap().unwrap();
    queue.submit([encoder.finish()]);
    let values = [[f32::NAN, 0.2, 0.3, 0.], [0.2, 0.18, 0.3, 8.]];
    queue.write_buffer(&source, 0, bytemuck::cast_slice(&values));
    let mut encoder = device.create_command_encoder(&Default::default());
    let good = binding.encode(&device, &mut encoder).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&bad, &good));
    drop(binding);
    drop(look);
    drop(renderer);
    let submission = queue.submit([encoder.finish()]);
    bad.begin(|| {});
    good.begin(|| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    assert!(
        bad.result()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("first rejected sample 1")
    );
    good.result().unwrap().unwrap();
    let actual = read_texture(&device, &queue, &output);
    assert_eq!(actual[0], [0.; 4]);
    assert_eq!(actual[1][3], 1.);
    assert!(actual[1][..3].iter().all(|v| v.is_finite()));
}

#[test]
fn display_attachment_admission_checks_format_extent_and_complete_grid() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: [0, 0, 3, 2].into(),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let reference = Reference::capture(
        &device,
        &renderer,
        OutputReference::DisplayReferred(OutputTarget::Srgb),
        None,
    )
    .unwrap();
    let budget = Budget {
        tile: [3, 2],
        scratch_bytes: 1 << 20,
    };
    let wrong_size = texture(&device, [2, 2], wgpu::TextureFormat::Rgba32Float);
    assert!(
        reference
            .presentation(borrowed_input(domain), &renderer, &wrong_size)
            .is_err()
    );
    let wrong_format = texture(&device, [3, 2], wgpu::TextureFormat::Rgba16Float);
    let image = reference
        .presentation(borrowed_input(domain), &renderer, &wrong_format)
        .unwrap();
    let plan = Plan::new(image, domain.bounds, budget).unwrap();
    assert!(Executor::admit(&device.limits(), &plan).is_err());
    let output = texture(&device, [3, 2], wgpu::TextureFormat::Rgba32Float);
    let image = reference
        .presentation(borrowed_input(domain), &renderer, &output)
        .unwrap();
    let evaluation = renderer
        .evaluation
        .request(
            image.clone(),
            Region {
                bounds: domain.bounds,
                grid: domain.grid,
            },
            budget,
        )
        .unwrap();
    let error = renderer
        .evaluation
        .evaluate(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |_, _, _| panic!("a texture cannot be delivered as a buffer"),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        Error::InvalidParameter("buffer sink requires raster storage").to_string()
    );
    let recorded = renderer.evaluation.record(evaluation, vec![]).unwrap();
    let pixels = buffer(&device, &[[0.; 4]; 6]);
    let mut encoder = device.create_command_encoder(&Default::default());
    let error = recorded
        .encode_into(&device, &mut encoder, &pixels, &pixels)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        Error::InvalidParameter("buffer sink requires raster storage").to_string()
    );
    assert!(Plan::new(image.clone(), [0, 0, 2, 1].into(), budget).is_err());
    assert!(
        Plan::new(
            image,
            domain.bounds,
            Budget {
                tile: [2, 1],
                ..budget
            }
        )
        .is_err()
    );
    let scene = Reference::capture(
        &device,
        &renderer,
        OutputReference::SceneReferred,
        Some(&asset()),
    )
    .unwrap();
    assert!(scene.look().is_none());
    let input = borrowed_input(domain);
    assert!(Arc::ptr_eq(
        &input,
        &scene.compose(input.clone(), &renderer).unwrap()
    ));
}
