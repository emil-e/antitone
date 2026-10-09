#![cfg(all(feature = "ui", feature = "render"))]
use antitone::{
    color::{Balance, ap0_to_ap1, ap1_to_ap0},
    gpu::ProcessingParameters,
    processing::{Calibration, PhotoSettings, Pipeline, Response, RollSettings},
    rendering::{CpuRenderer, OutputTarget},
    viewer::GpuImage,
};
use glam::{DMat3, DVec3};

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
            .map(|p| matrix * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.))
            .collect();
        assert!(scans.iter().any(|p| p.min_element() < 0.));
        assert!(scans.iter().any(|p| p.max_element() > 1.));
        let working: Vec<_> = scans.iter().map(|p| ap0_to_ap1() * *p).collect();
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
        let pipeline = Pipeline::new(calibration, roll, photo).unwrap();
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
            let pipeline = Pipeline::new(calibration, settings, photo).unwrap();
            let values: Vec<_> = scans
                .iter()
                .map(|s| pipeline.apply(*s).unwrap().rgb)
                .collect();
            (
                ProcessingParameters::positive(calibration, settings, photo).unwrap(),
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
                    ProcessingParameters::positive(calibration, roll, photo).unwrap(),
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
            .map(|p| {
                ap0_to_ap1() * matrix * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.)
            })
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
