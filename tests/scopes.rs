#![cfg(all(feature = "ui", feature = "render"))]
use antitone::{
    frame_raster::FrameRaster,
    geometry::{Composition, SourceRegion},
    gpu::ProcessingParameters,
    gpu_scopes::{GpuScopes, ScopePaint},
    rendering::OutputTarget,
    scopes::{self, PLANE_LEN, Plot, Settings},
    viewer::GpuImage,
};
use egui_wgpu::CallbackTrait;
use glam::DMat3;
use std::sync::Arc;

#[path = "scopes/geometry.rs"]
mod geometry;
#[path = "scopes/shared_renderer.rs"]
mod shared_renderer;

fn full_frame(size: [u32; 2]) -> FrameRaster {
    FrameRaster::full(
        Composition::new(
            SourceRegion {
                x: 0,
                y: 0,
                width: size[0],
                height: size[1],
            },
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap()
}

async fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    adapter.request_device(&Default::default()).await.unwrap()
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: [u32; 2],
    pixels: &[[f32; 4]],
) -> wgpu::Texture {
    let image = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scalar scope fixture"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    queue.write_texture(
        image.as_image_copy(),
        bytemuck::cast_slice(pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 16),
            rows_per_image: Some(size[1]),
        },
        image.size(),
    );
    image
}

fn read(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Vec<u32> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap();
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let output = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    staging.unmap();
    output
}

fn paint_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    paint: impl FnOnce(&mut wgpu::RenderPass<'static>),
) -> Vec<[f32; 4]> {
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scope presentation regression"),
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = output.create_view(&Default::default());
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        paint(&mut pass.forget_lifetime());
    }
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * 256 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 8),
                rows_per_image: Some(256),
            },
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap();
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let pixels = bytemuck::cast_slice::<u8, u16>(&mapped)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|rgba| std::array::from_fn(|c| half::f16::from_bits(rgba[c]).to_f32()))
        .collect();
    drop(mapped);
    staging.unmap();
    pixels
}

#[test]
fn grading_and_viewport_edits_keep_a_trace_but_other_sources_and_graticules_do_not() {
    #[derive(Clone, Copy)]
    enum Change {
        Grade,
        Viewport,
        Sampling,
        Target,
        Plot,
        Amplification,
        Source,
        SourceRevision,
        Composition,
        Suspend,
    }
    pollster::block_on(async {
        let (device, queue) = device().await;
        let context = egui::Context::default();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [256, 256],
            pixels_per_point: 1.,
        };
        let mut resources = egui_wgpu::CallbackResources::default();
        for change in [
            Change::Grade,
            Change::Viewport,
            Change::Sampling,
            Change::Target,
            Change::Plot,
            Change::Amplification,
            Change::Source,
            Change::SourceRevision,
            Change::Composition,
            Change::Suspend,
        ] {
            let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
            let image = Arc::new(
                GpuImage::new(
                    &device,
                    &queue,
                    7,
                    3,
                    &[[2000, 3000, 4000, 0]; 21],
                    DMat3::IDENTITY,
                    OutputTarget::DisplayP3,
                )
                .await
                .unwrap(),
            );
            let settings = Settings {
                plot: Plot::Vector,
                ..Default::default()
            };
            let parameters = ProcessingParameters::source();
            let first = ScopePaint::new(
                gpu.clone(),
                image.clone(),
                parameters,
                full_frame([7, 3]),
                settings,
                context.clone(),
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            first.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            let mut requested_image = image.clone();
            let mut requested_settings = settings;
            let mut requested_frame = full_frame([7, 3]);
            match change {
                Change::Grade => {}
                Change::Viewport => {
                    requested_frame = FrameRaster::new(
                        requested_frame.composition,
                        antitone::geometry::FramePoint([2., 1.]),
                        antitone::geometry::FramePoint([5., 3.]),
                        [3, 2],
                    )
                    .unwrap();
                }
                Change::Sampling => requested_frame.output = [3, 2],
                Change::Target => requested_settings.target = OutputTarget::Srgb,
                Change::Plot => requested_settings.plot = Plot::Rgb,
                Change::Amplification => requested_settings.amplification = 8,
                Change::Source => {
                    requested_image = Arc::new(
                        GpuImage::new(
                            &device,
                            &queue,
                            7,
                            3,
                            &[[2000, 3000, 4000, 0]; 21],
                            DMat3::IDENTITY,
                            OutputTarget::DisplayP3,
                        )
                        .await
                        .unwrap(),
                    )
                }
                Change::SourceRevision => image
                    .write_source(&queue, &[[4000, 5000, 6000, 0]; 21])
                    .unwrap(),
                Change::Composition => {
                    let mut frame = requested_frame;
                    frame.composition = Composition::new(
                        frame.composition.area(),
                        Default::default(),
                        antitone::geometry::PhotoOrientation {
                            turns: antitone::geometry::QuarterTurns::Two,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    requested_frame = frame;
                }
                Change::Suspend => gpu.invalidate(),
            }
            let latest_parameters = if matches!(change, Change::Grade) {
                ProcessingParameters::source_inspection(0.2).unwrap()
            } else {
                parameters
            };
            let latest = ScopePaint::new(
                gpu.clone(),
                requested_image.clone(),
                latest_parameters,
                requested_frame,
                requested_settings,
                context.clone(),
            );
            latest.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            assert_eq!(gpu.reduction_count(), 1, "retain the outstanding-job bound");
            assert!(
                gpu.status(
                    &requested_image,
                    latest_parameters,
                    requested_frame,
                    requested_settings
                )
                .is_none(),
                "held bins must not be reported as current diagnostics"
            );
            let pixels = paint_pixels(&device, &queue, encoder, |pass| {
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256., 256.));
                latest.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.,
                        screen_size_px: [256, 256],
                    },
                    pass,
                    &resources,
                );
            });
            if matches!(change, Change::Grade | Change::Viewport | Change::Sampling) {
                assert!(
                    pixels.iter().any(|rgb| rgb[..3].iter().all(|&v| v > 0.9)),
                    "grading/navigation must not blank the held trace"
                );
                assert!(
                    gpu.status(
                        &image,
                        latest_parameters,
                        requested_frame,
                        requested_settings
                    )
                    .is_none(),
                    "old completion must not become the latest grade/region"
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                latest.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                assert_eq!(gpu.reduction_count(), 2);
                assert_eq!(
                    gpu.status(
                        &image,
                        latest_parameters,
                        requested_frame,
                        requested_settings
                    )
                    .unwrap()
                    .unwrap()
                    .valid,
                    requested_frame.output[0] * requested_frame.output[1]
                );
            } else {
                assert!(
                    pixels.iter().all(|rgba| rgba[..3] == [0.; 3]),
                    "incompatible bins must not paint"
                );
            }
        }
    });
}

#[test]
fn gpu_waveform_matches_scalar_bins_columns_endpoints_and_invalid_counts() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let gpu = GpuScopes::new(&device).await.unwrap();
        let size = [17, 9];
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let mut pixels: Vec<_> = (0..size[0] * size[1])
                .map(|i| {
                    std::array::from_fn(|c| {
                        if c == 3 {
                            1.
                        } else {
                            target
                                .decode(
                                    ((i * (13 + c as u32 * 6)) % 256) as f64 / 256. + 0.5 / 256.,
                                )
                                .unwrap() as f32
                        }
                    })
                })
                .collect();
            pixels[0] = [0., 0., 0., 1.];
            pixels[1] = [1., 1., 1., 1.];
            pixels[2] = [f32::NAN, 0.1, 0.2, 1.];
            pixels[3] = [f32::INFINITY, 0., 0., 1.];
            pixels[4] = [-0.01, 0., 0., 1.];
            pixels[5] = [1.01, 0., 0., 1.];
            let image = texture(&device, &queue, size, &pixels);
            for plot in [Plot::Rgb, Plot::Luma] {
                let settings = Settings {
                    target,
                    plot,
                    amplification: 1,
                };
                let (expected, columns, counts) =
                    scopes::reference(size, &pixels, settings).unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                gpu.encode(&device, &mut encoder, &image, settings).unwrap();
                queue.submit([encoder.finish()]);
                let actual = read(&device, &queue, &gpu.bins);
                let totals = read(&device, &queue, &gpu.totals);
                assert_eq!(
                    &totals[..3],
                    &[counts.valid, counts.invalid, counts.outside]
                );
                assert_eq!(&totals[4..], columns);
                assert_eq!(counts.invalid, 4);
                assert_eq!(actual, expected, "{target:?} {plot:?}");
                for plane in actual.chunks(PLANE_LEN) {
                    assert_eq!(plane.iter().sum::<u32>(), counts.valid);
                }
            }
        }
    });
}

#[test]
fn gpu_vector_projection_amplification_and_boundary_tolerance_match_scalar() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let gpu = GpuScopes::new(&device).await.unwrap();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            for gain in [1, 2, 4, 8] {
                for rgb in [
                    [0., 0., 0.],
                    [1., 1., 1.],
                    [0.18, 0.18, 0.18],
                    [0.2, 0.18, 0.18],
                    [1., 0., 0.],
                    [1., 0., 1.],
                    [0., 0., 1.],
                    [0., 1., 1.],
                    [0., 1., 0.],
                    [1., 1., 0.],
                ] {
                    let pixels = [[rgb[0], rgb[1], rgb[2], 1.]];
                    let image = texture(&device, &queue, [1, 1], &pixels);
                    let settings = Settings {
                        target,
                        plot: Plot::Vector,
                        amplification: gain,
                    };
                    let (expected, _, counts) =
                        scopes::reference([1, 1], &pixels, settings).unwrap();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    gpu.encode(&device, &mut encoder, &image, settings).unwrap();
                    queue.submit([encoder.finish()]);
                    let actual = read(&device, &queue, &gpu.bins);
                    let totals = read(&device, &queue, &gpu.totals);
                    assert_eq!(
                        &totals[..3],
                        &[counts.valid, counts.invalid, counts.outside]
                    );
                    assert_eq!(actual.iter().sum::<u32>() + totals[2], 1);
                    if counts.outside == 0 {
                        let a = actual.iter().position(|&count| count == 1).unwrap();
                        let b = expected.iter().position(|&count| count == 1).unwrap();
                        // Neutral/primary coordinates can land on a bin edge; float rounding may
                        // choose the adjacent bin without weakening the production colour gate.
                        assert!(
                            (a % 256).abs_diff(b % 256) <= 1 && (a / 256).abs_diff(b / 256) <= 1,
                            "{target:?} {gain}× {rgb:?}: {a} / {b}"
                        );
                    }
                }
            }
        }
    });
}

#[test]
fn shared_preparation_and_scope_cache_reuse_rendering_and_reject_old_diagnostics() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = antitone::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let image = Arc::new(
            GpuImage::new_shared(
                &renderer,
                &device,
                7,
                3,
                &[[2000, 3000, 4000, 0]; 21],
                DMat3::IDENTITY,
                OutputTarget::DisplayP3,
            )
            .await
            .unwrap(),
        );
        let parameters = ProcessingParameters::source();
        let context = egui::Context::default();
        let settings = Settings {
            plot: Plot::Vector,
            ..Default::default()
        };
        let mut resources = egui_wgpu::CallbackResources::default();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [600, 400],
            pixels_per_point: 2.,
        };
        let paint = |settings| {
            ScopePaint::new(
                gpu.clone(),
                image.clone(),
                parameters,
                full_frame([7, 3]),
                settings,
                context.clone(),
            )
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(settings).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(
            !image.prepare(&device, &mut encoder, parameters),
            "viewer must reuse scope preparation"
        );
        paint(settings).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert_eq!(gpu.reduction_count(), 1);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(
            gpu.status(&image, parameters, full_frame([7, 3]), settings)
                .unwrap()
                .unwrap()
                .valid,
            21
        );
        let zoom = Settings {
            amplification: 8,
            ..settings
        };
        assert!(
            gpu.status(&image, parameters, full_frame([7, 3]), zoom)
                .is_none()
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(zoom).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(
            !image.prepare(&device, &mut encoder, parameters),
            "scope zoom must not rerender ACES"
        );
        gpu.invalidate();
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            gpu.status(&image, parameters, full_frame([7, 3]), zoom)
                .is_none(),
            "late diagnostics must not reinstall after suspension"
        );
        image
            .write_source(&queue, &[[4000, 5000, 6000, 0]; 21])
            .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(image.prepare(&device, &mut encoder, parameters));
        paint(settings).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(gpu.reduction_count(), 3);
        let other = Arc::new(
            GpuImage::new_shared(
                &renderer,
                &device,
                7,
                3,
                &[[2000, 3000, 4000, 0]; 21],
                DMat3::IDENTITY,
                OutputTarget::DisplayP3,
            )
            .await
            .unwrap(),
        );
        assert!(
            gpu.status(&other, parameters, full_frame([7, 3]), settings)
                .is_none()
        );
        image
            .write_source(&queue, &[[5000, 6000, 7000, 0]; 21])
            .unwrap();
        assert!(
            gpu.status(&image, parameters, full_frame([7, 3]), settings)
                .is_none(),
            "source writes must invalidate same-image scope identity"
        );
        let bad = Settings {
            amplification: 3,
            ..settings
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(bad).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(
            gpu.status(&image, parameters, full_frame([7, 3]), bad)
                .unwrap()
                .is_err()
        );
        let failed_count = gpu.reduction_count();
        paint(bad).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert_eq!(gpu.reduction_count(), failed_count);
        queue.submit([encoder.finish()]);
    });
}

#[test]
fn vector_intensity_tracks_chroma_density_not_zoomed_bin_occupancy() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let gpu = GpuScopes::new(&device).await.unwrap();
        let input = texture(&device, &queue, [1, 1], &[[0.18, 0.18, 0.18, 1.]]);
        let index = 128 * 256 + 128;
        let valid = 1_048_576;
        let expected = (1. + 256. / f64::from(valid) * 10000.).log2() * 0.18;
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            for amplification in [1_u32, 2, 4, 8] {
                let settings = Settings {
                    target,
                    plot: Plot::Vector,
                    amplification,
                };
                let mut encoder = device.create_command_encoder(&Default::default());
                let group = gpu.encode(&device, &mut encoder, &input, settings).unwrap();
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                // Synthetic counters isolate the display transfer from sample/bin-edge rounding.
                // Equal physical density occupies gain² more bins, reducing each bin's count.
                let mut bins = vec![0_u32; PLANE_LEN * 4];
                bins[index] = 256 / amplification.pow(2);
                queue.write_buffer(&gpu.bins, 0, bytemuck::cast_slice(&bins));
                for outside in [0, valid / 2] {
                    queue.write_buffer(
                        &gpu.totals,
                        0,
                        bytemuck::cast_slice(&[valid, 0, outside, 0]),
                    );
                    let pixels = paint_pixels(
                        &device,
                        &queue,
                        device.create_command_encoder(&Default::default()),
                        |pass| gpu.paint(pass, &group),
                    );
                    for (c, tint) in [0.93, 0.96, 1.].into_iter().enumerate() {
                        assert!(
                            (f64::from(pixels[index][c]) - expected * tint).abs() < 0.001,
                            "{target:?} {amplification}× outside={outside}: {:?}",
                            pixels[index]
                        );
                    }
                    assert!(
                        pixels[index][2] > 0.3,
                        "default intensity must remain useful"
                    );
                }
            }
        }
    });
}

#[test]
fn scope_density_paints_nonempty_encoded_sdr() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let gpu = GpuScopes::new(&device).await.unwrap();
        let input = texture(&device, &queue, [1, 1], &[[0.18, 0.18, 0.18, 1.]]);
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        for plot in [Plot::Rgb, Plot::Luma, Plot::Vector] {
            let settings = Settings {
                plot,
                ..Default::default()
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            let group = gpu.encode(&device, &mut encoder, &input, settings).unwrap();
            let view = output.create_view(&Default::default());
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                gpu.paint(&mut pass.forget_lifetime(), &group);
            }
            queue.submit([encoder.finish()]);
            let counts = read(&device, &queue, &gpu.bins);
            let index = counts.iter().position(|&count| count == 1).unwrap() % PLANE_LEN;
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    origin: wgpu::Origin3d {
                        x: index as u32 % 256,
                        y: index as u32 / 256,
                        z: 0,
                    },
                    ..output.as_image_copy()
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (send, recv) = std::sync::mpsc::channel();
            staging.map_async(wgpu::MapMode::Read, .., move |result| {
                send.send(result).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            recv.recv().unwrap().unwrap();
            let mapped = staging.get_mapped_range(..).unwrap();
            let rgb: Vec<_> = bytemuck::cast_slice::<u8, u16>(&mapped[..6])
                .iter()
                .map(|&bits| half::f16::from_bits(bits).to_f32())
                .collect();
            assert!(
                rgb.iter().all(|&value| value > 0.5 && value <= 1.),
                "{plot:?}: {rgb:?}"
            );
            drop(mapped);
            staging.unmap();
        }
    });
}

#[test]
fn rapid_requests_and_hide_show_coalesce_until_completion_and_luma_reuses_bins() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let image = Arc::new(
            GpuImage::new(
                &device,
                &queue,
                7,
                3,
                &[[2000, 3000, 4000, 0]; 21],
                DMat3::IDENTITY,
                OutputTarget::DisplayP3,
            )
            .await
            .unwrap(),
        );
        let parameters = ProcessingParameters::source();
        let context = egui::Context::default();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [600, 400],
            pixels_per_point: 2.,
        };
        let mut resources = egui_wgpu::CallbackResources::default();
        let paint = |settings| {
            ScopePaint::new(
                gpu.clone(),
                image.clone(),
                parameters,
                full_frame([7, 3]),
                settings,
                context.clone(),
            )
        };
        let waveform = Settings::default();
        let latest = Settings {
            plot: Plot::Vector,
            amplification: 8,
            ..waveform
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(waveform).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        for amplification in [1, 2, 4, 8, 4, 2, 8] {
            paint(Settings {
                amplification,
                ..latest
            })
            .prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        }
        assert_eq!(gpu.reduction_count(), 1);
        gpu.invalidate();
        paint(latest).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert_eq!(
            gpu.reduction_count(),
            1,
            "hide/show must not bypass the outstanding-job bound"
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            gpu.status(&image, parameters, full_frame([7, 3]), waveform)
                .is_none()
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(latest).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(gpu.reduction_count(), 2);
        assert_eq!(
            gpu.status(&image, parameters, full_frame([7, 3]), latest)
                .unwrap()
                .unwrap()
                .valid,
            21
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(waveform).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let luma = Settings {
            plot: Plot::Luma,
            ..waveform
        };
        let before = gpu.reduction_count();
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(luma).prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert_eq!(gpu.reduction_count(), before);
        assert_eq!(
            gpu.status(&image, parameters, full_frame([7, 3]), luma),
            gpu.status(&image, parameters, full_frame([7, 3]), waveform)
        );
        queue.submit([encoder.finish()]);
    });
}
