#![cfg(all(feature = "ui", feature = "render"))]
use antitone::{
    color::{Balance, ap1_to_ap0},
    flat_field::FlatField,
    gpu::{GpuFlatField, ProcessingParameters, SourceGeometry},
    processing::{Calibration, PhotoSettings, Pipeline, Response, RollSettings},
    rendering::{CpuRenderer, OutputTarget},
    viewer::GpuImage,
};
use glam::{DMat3, DVec3};

#[test]
fn corrected_viewer_shares_the_field_and_matches_both_cpu_output_targets() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let matrix = DMat3::from_cols(
            DVec3::new(0.6, 0.02, -0.01),
            DVec3::new(0.4, 1.2, -0.15),
            DVec3::new(0., -0.22, 1.16),
        );
        let field = std::sync::Arc::new(
            FlatField::from_nodes(
                [46, 46, 8602, 5508],
                [2, 2],
                vec![
                    DVec3::new(0.9, 1., 0.95),
                    DVec3::new(1.2, 1.1, 1.),
                    DVec3::new(1., 1.2, 0.9),
                    DVec3::new(1.3, 1.15, 1.1),
                ],
            )
            .unwrap(),
        );
        let gpu = std::sync::Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
        let geometry = SourceGeometry::new([12, 12, 8640, 5760], [8, 8]).unwrap();
        let pixels: Vec<_> = (0..64u32)
            .map(|i| {
                [
                    (i * 12347) as u16,
                    (i * 19531) as u16,
                    (i * 9109) as u16,
                    65535,
                ]
            })
            .collect();
        let scans: Vec<_> = pixels
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let [x, y] = geometry.pixel(i as u32).unwrap();
                DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.
                    * field.gain([x as f64 + 0.5, y as f64 + 0.5]).unwrap()
            })
            .collect();
        let base = Calibration {
            dmin: DVec3::new(0.28, 0.23, 0.25),
            dmax: DVec3::new(0.01, 0.005, 0.001),
            input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
        };
        let calibration = field.processing_calibration(base).unwrap();
        let mut cases = vec![(
            ProcessingParameters::source(),
            scans.iter().map(|p| matrix * p).collect::<Vec<_>>(),
        )];
        for response in [Response::Original, Response::film()] {
            let roll = RollSettings {
                response,
                exposure: -2.,
                ..RollSettings::default()
            };
            let pipeline =
                Pipeline::new(calibration, roll, PhotoSettings::default(), matrix).unwrap();
            cases.push((
                ProcessingParameters::positive(calibration, roll, PhotoSettings::default(), matrix)
                    .unwrap(),
                scans
                    .iter()
                    .map(|p| pipeline.apply(*p).unwrap().rgb)
                    .collect(),
            ));
        }
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let mut image = GpuImage::new(&device, &queue, 8, 8, &pixels, matrix, target)
                .await
                .unwrap();
            image.set_flat_field(Some(gpu.clone()), geometry).unwrap();
            assert_eq!(image.processing_byte_len(), (64 * 8, 64 * 16));
            for (parameters, working) in &cases {
                let mut encoder = device.create_command_encoder(&Default::default());
                image.encode(&device, &mut encoder, *parameters);
                queue.submit([encoder.finish()]);
                let expected = CpuRenderer::new(target).unwrap().render(working).unwrap();
                let actual = read(&device, &queue, &image.linear).await;
                for (a, b) in actual.iter().zip(expected) {
                    for c in 0..3 {
                        assert!(
                            (a[c] - b[c]).abs() < 2e-5,
                            "target={target:?} actual={a:?} expected={b:?}"
                        );
                    }
                }
            }
            image.set_flat_field(None, geometry).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            image.encode(&device, &mut encoder, ProcessingParameters::source());
            queue.submit([encoder.finish()]);
            let actual = read(&device, &queue, &image.linear).await;
            let uncorrected: Vec<_> = pixels
                .iter()
                .map(|p| matrix * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.))
                .collect();
            let expected = CpuRenderer::new(target)
                .unwrap()
                .render(&uncorrected)
                .unwrap();
            for (a, b) in actual.iter().zip(expected) {
                for c in 0..3 {
                    assert!((a[c] - b[c]).abs() < 2e-5);
                }
            }
        }
    });
}

#[test]
fn reusable_gpu_viewer_matches_cpu_targets_and_extended_srgb_presentation() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let matrix = DMat3::from_cols(
            DVec3::new(3., 0., -0.6),
            DVec3::new(-0.9, 3., 0.),
            DVec3::new(0., 0., 4.5),
        );
        let pixels: Vec<[u16; 4]> = [
            DVec3::splat(0.18),
            DVec3::new(0.5, 0.01, 0.04),
            DVec3::new(0.01, 0.8, 0.2),
            DVec3::new(0.01, 0.1, 1.5),
            DVec3::ZERO,
            DVec3::ONE,
            DVec3::new(-0.01, 0.4, 0.2),
            DVec3::new(2., 0.8, 0.1),
        ]
        .into_iter()
        .map(|p| {
            let camera = matrix.inverse() * p;
            assert!(camera.min_element() >= 0. && camera.max_element() <= 1.);
            [
                (camera.x * 65535.).round() as u16,
                (camera.y * 65535.).round() as u16,
                (camera.z * 65535.).round() as u16,
                0,
            ]
        })
        .collect();
        let scans: Vec<_> = pixels
            .iter()
            .map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.)
            .collect();
        let working: Vec<_> = scans.iter().map(|p| matrix * *p).collect();
        assert!(working.iter().any(|p| p.min_element() < 0.));
        assert!(working.iter().any(|p| p.max_element() > 1.));
        let calibration = Calibration {
            dmin: DVec3::new(0.2, 0.15, 0.1),
            dmax: DVec3::new(0.008, 0.005, 0.002),
            input_floor: 2.2e-5,
        };
        let roll = RollSettings {
            gamma: 0.65,
            balance: Balance {
                warmth: 0.2,
                tint: -0.1,
            },
            exposure: -2.,
            ..RollSettings::default()
        };
        let photo = PhotoSettings {
            wb: Balance {
                warmth: -0.1,
                tint: 0.2,
            },
            exposure: 0.5,
        };
        let pipeline = Pipeline::new(calibration, roll, photo, matrix).unwrap();
        let positive: Vec<_> = scans
            .iter()
            .map(|p| pipeline.apply(*p).unwrap().rgb)
            .collect();
        let films: Vec<_> = [
            Response::film(),
            Response::Film {
                toe: 3.875,
                shoulder: 0.02,
            },
        ]
        .map(|response| {
            let settings = RollSettings { response, ..roll };
            let pipeline = Pipeline::new(calibration, settings, photo, matrix).unwrap();
            let values: Vec<_> = scans
                .iter()
                .map(|s| pipeline.apply(*s).unwrap().rgb)
                .collect();
            (
                ProcessingParameters::positive(calibration, settings, photo, matrix).unwrap(),
                values,
            )
        })
        .into();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let image = GpuImage::new(&device, &queue, 4, 2, &pixels, matrix, target)
                .await
                .unwrap();
            assert_eq!(
                image.processing_byte_len(),
                (pixels.len() as u64 * 8, pixels.len() as u64 * 16)
            );
            assert!(image.write_source(&queue, &pixels[..7]).is_err());
            let rendered = GpuImage::new_rendered(&device, &queue, 4, 2, target)
                .await
                .unwrap();
            assert_eq!(rendered.processing_byte_len(), (0, 0));
            assert!(rendered.write_source(&queue, &pixels).is_err());
            for (parameters, working) in [
                (ProcessingParameters::source(), &working),
                (
                    ProcessingParameters::positive(calibration, roll, photo, matrix).unwrap(),
                    &positive,
                ),
                (films[0].0, &films[0].1),
                (films[1].0, &films[1].1),
            ] {
                let mut encoder = device.create_command_encoder(&Default::default());
                image.encode(&device, &mut encoder, parameters);
                queue.submit([encoder.finish()]);
                let expected = CpuRenderer::new(target).unwrap().render(working).unwrap();
                let actual = read(&device, &queue, &image.linear).await;
                for (actual, expected) in actual.iter().zip(&expected) {
                    for c in 0..3 {
                        assert!(
                            (actual[c] - expected[c]).abs() < 2e-5,
                            "{target:?} {actual:?} {expected:?}"
                        );
                    }
                }
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 4,
                        height: 2,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                for (uv, indices) in [
                    ([0., 0., 1., 1.], [0, 1, 2, 3, 4, 5, 6, 7]),
                    ([0.25, 0., 0.5, 1.], [1, 1, 2, 2, 5, 5, 6, 6]),
                ] {
                    let group = image.presentation_group(&device, uv);
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
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
                        image.paint_group(&mut pass.forget_lifetime(), &group);
                    }
                    queue.submit([encoder.finish()]);
                    let output = read(&device, &queue, &texture).await;
                    for (actual, index) in output.iter().zip(indices) {
                        let linear = expected[index];
                        let srgb = target.linear_to_srgb() * linear.as_dvec3();
                        for c in 0..3 {
                            let x = srgb[c];
                            let a = x.abs();
                            let expected = x.signum()
                                * if a <= 0.0031308 {
                                    12.92 * a
                                } else {
                                    1.055 * a.powf(1. / 2.4) - 0.055
                                };
                            assert!(
                                (f64::from(actual[c]) - expected).abs() < 0.0015,
                                "{target:?} {actual:?} {srgb:?}"
                            );
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn gpu_yellows_retain_hue_and_continuity_across_exposure() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut inputs = Vec::new();
        for blue in [0., 0.25, 0.5] {
            for i in 0..=320 {
                inputs.push(DVec3::new(1., 1., blue) * 2f64.powf(-6. + f64::from(i) / 32.));
            }
        }
        let matrix = ap1_to_ap0() * 64.;
        let pixels: Vec<[u16; 4]> = inputs
            .iter()
            .map(|input| {
                let camera = *input / 64.;
                [
                    (camera.x * 65535.).round() as u16,
                    (camera.y * 65535.).round() as u16,
                    (camera.z * 65535.).round() as u16,
                    0,
                ]
            })
            .collect();
        let inputs: Vec<_> = pixels
            .iter()
            .map(|p| matrix * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.))
            .collect();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let image = GpuImage::new(&device, &queue, 321, 3, &pixels, matrix, target)
                .await
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            image.encode(&device, &mut encoder, ProcessingParameters::source());
            queue.submit([encoder.finish()]);
            let actual = read(&device, &queue, &image.linear).await;
            let expected = CpuRenderer::new(target).unwrap().render(&inputs).unwrap();
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    actual[0].min(actual[1]) > actual[2] + 1e-6,
                    "{target:?} lost yellow: {actual:?}"
                );
                for c in 0..3 {
                    assert!(
                        (actual[c] - expected[c]).abs() < 2e-5,
                        "{target:?} {actual:?} vs {expected:?}"
                    );
                }
            }
            for ramp in actual.chunks(321) {
                for pair in ramp.windows(2) {
                    for c in 0..3 {
                        assert!(
                            (pair[1][c] - pair[0][c]).abs() < 0.025,
                            "{target:?} exposure discontinuity: {pair:?}"
                        );
                    }
                }
            }
        }
    });
}

#[test]
fn tiled_linear_area_average_matches_scalar_fractional_footprints() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let pixels: Vec<[f32; 4]> = (0..15 * 9)
            .map(|i| {
                [
                    ((i * 13) % 31) as f32 / 15. - 0.2,
                    ((i * 7) % 23) as f32 / 13.,
                    ((i * 19) % 41) as f32 / 23.,
                    1.,
                ]
            })
            .collect();
        let texture = |width, height, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage,
                view_formats: &[],
            })
        };
        let tile = texture(
            6,
            4,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let output = texture(
            4,
            3,
            wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let average = antitone::resample::AreaAverage::new(&device, [15, 9], [4, 3])
            .await
            .unwrap();
        for y in (0..9).step_by(4) {
            for x in (0..15).step_by(6) {
                let mut samples = vec![[0.; 4]; 6 * 4];
                for dy in 0..4.min(9 - y) {
                    for dx in 0..6.min(15 - x) {
                        samples[(dy * 6 + dx) as usize] = pixels[((y + dy) * 15 + x + dx) as usize];
                    }
                }
                queue.write_texture(
                    tile.as_image_copy(),
                    bytemuck::cast_slice(&samples),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(6 * 16),
                        rows_per_image: Some(4),
                    },
                    tile.size(),
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                average.accumulate(&device, &mut encoder, &tile, [x, y]);
                queue.submit([encoder.finish()]);
            }
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        average.finish(&device, &mut encoder, &output);
        queue.submit([encoder.finish()]);
        let actual = read(&device, &queue, &output).await;
        for y in 0..3 {
            for x in 0..4 {
                let lo = [x as f64 * 15. / 4., y as f64 * 3.];
                let hi = [(x + 1) as f64 * 15. / 4., (y + 1) as f64 * 3.];
                let mut expected = [0f64; 3];
                for sy in 0..9 {
                    for sx in 0..15 {
                        let weight = (hi[0].min((sx + 1) as f64) - lo[0].max(sx as f64)).max(0.)
                            * (hi[1].min((sy + 1) as f64) - lo[1].max(sy as f64)).max(0.)
                            / (15. / 4. * 3.);
                        for c in 0..3 {
                            expected[c] += f64::from(pixels[sy * 15 + sx][c]) * weight;
                        }
                    }
                }
                for c in 0..3 {
                    assert!(
                        (f64::from(actual[y * 4 + x][c]) - expected[c]).abs() < 2e-6,
                        "{x},{y}: {:?} {expected:?}",
                        actual[y * 4 + x]
                    );
                }
            }
        }
    });
}

#[cfg(feature = "editor")]
#[test]
fn large_retina_fit_area_average_uses_supported_buffer_limits() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let source = antitone::raw::Rect {
            x: 12,
            y: 12,
            width: 8640,
            height: 5760,
        };
        let geometry = antitone::viewport::Viewport::default().layout(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2560., 1707.)),
            source,
            2.,
        );
        let region = geometry.visible_area();
        assert_eq!(region, source);
        let output = geometry.output_size(region, 2.);
        assert_eq!(output, [5120, 3414]);
        let size = u64::from(output[0]) * u64::from(output[1]) * 16;
        assert!(size > wgpu::Limits::default().max_storage_buffer_binding_size);
        assert!(size > wgpu::Limits::default().max_buffer_size);
        let (default_device, _) = adapter.request_device(&Default::default()).await.unwrap();
        assert!(matches!(
            antitone::resample::AreaAverage::new(
                &default_device,
                [source.width, source.height],
                output,
            )
            .await,
            Err(antitone::Error::Gpu(message))
                if message == "invalid area-average dimensions or GPU budget"
        ));
        drop(default_device);
        let limits = antitone::platform::device_limits(adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(device.limits(), limits);
        let average =
            antitone::resample::AreaAverage::new(&device, [source.width, source.height], output)
                .await
                .unwrap();
        for (source, output) in [([0, 9], [4, 3]), ([15, 9], [0, 3]), ([15, 9], [16, 3])] {
            assert!(
                antitone::resample::AreaAverage::new(&device, source, output)
                    .await
                    .is_err()
            );
        }
        let texture = |width, height, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage,
                view_formats: &[],
            })
        };
        let tile = texture(
            1,
            1,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let target = texture(
            output[0],
            output[1],
            wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let rgb = [0.25f32, 0.5, 0.75, 1.];
        queue.write_texture(
            tile.as_image_copy(),
            bytemuck::cast_slice(&rgb),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: Some(1),
            },
            tile.size(),
        );
        let samples = texture(
            3,
            1,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        average.accumulate(&device, &mut encoder, &tile, [0, 0]);
        average.accumulate(
            &device,
            &mut encoder,
            &tile,
            [source.width - 1, source.height - 1],
        );
        average.finish(&device, &mut encoder, &target);
        for (i, [x, y]) in [
            [0, 0],
            [output[0] / 2, output[1] / 2],
            [output[0] - 1, output[1] - 1],
        ]
        .into_iter()
        .enumerate()
        {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    ..target.as_image_copy()
                },
                wgpu::TexelCopyTextureInfo {
                    origin: wgpu::Origin3d {
                        x: i as u32,
                        y: 0,
                        z: 0,
                    },
                    ..samples.as_image_copy()
                },
                tile.size(),
            );
        }
        queue.submit([encoder.finish()]);
        let actual = read(&device, &queue, &samples).await;
        let footprint = f64::from(source.width) / f64::from(output[0])
            * (f64::from(source.height) / f64::from(output[1]));
        for (i, pixel) in actual.iter().enumerate() {
            for (c, (&actual, &input)) in pixel.iter().zip(&rgb).enumerate() {
                let expected = if c == 3 {
                    1.
                } else if i == 1 {
                    0.
                } else {
                    f64::from(input) / footprint
                };
                assert!(
                    (f64::from(actual) - expected).abs() < 2e-6,
                    "{i},{c}: {actual} vs {expected}"
                );
            }
        }
    });
}

async fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    let component = if texture.format() == wgpu::TextureFormat::Rgba16Float {
        2
    } else {
        4
    };
    let row = (texture.width() * component * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap()
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.get_mapped_range(..).unwrap();
    let mut pixels = Vec::new();
    for y in 0..texture.height() as usize {
        let bytes = &mapped[y * row as usize
            ..y * row as usize + texture.width() as usize * component as usize * 4];
        for pixel in bytes.chunks_exact(component as usize * 4) {
            pixels.push(std::array::from_fn(|c| {
                if component == 2 {
                    half::f16::from_bits(u16::from_le_bytes(
                        pixel[c * 2..c * 2 + 2].try_into().unwrap(),
                    ))
                    .to_f32()
                } else {
                    f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap())
                }
            }));
        }
    }
    drop(mapped);
    buffer.unmap();
    pixels
}
