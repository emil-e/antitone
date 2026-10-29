#![cfg(feature = "ui")]
use antitone::{
    crop::Crop,
    frame_raster::FrameRaster,
    geometry::{Composition, FramePoint, PhotoOrientation, RollOrientation, SourceRegion},
    resample::AreaAverage,
};

#[test]
fn gpu_rotated_box_footprints_match_scalar_coverage_across_tiles() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let area = SourceRegion {
            x: 120,
            y: 73,
            width: 23,
            height: 19,
        };
        let pixel = |x: u32, y: u32| {
            [
                (x * 13 + y * 7) % 31,
                (x * 7 + y * 11) % 23,
                (x * 19 + y * 3) % 41,
            ]
            .map(|v| v as f32 / 11. - 0.6)
        };
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
        for angle in [-89_f64, -45., -13.7, 0., 19.3, 44., 89.] {
            let crop = Crop {
                center: [132.25, 82.75],
                size: [15, 9],
                angle: angle.to_radians(),
            }
            .constrained(area)
            .unwrap();
            let composition = Composition::with_crop(
                area,
                RollOrientation {
                    horizontal: true,
                    ..Default::default()
                },
                PhotoOrientation::default(),
                Some(crop),
            )
            .unwrap();
            for output in [composition.size(), [4, 3], [1, 1]] {
                let raster = FrameRaster::new(
                    composition,
                    FramePoint([0., 0.]),
                    FramePoint(composition.size().map(f64::from)),
                    output,
                )
                .unwrap();
                let bounds = raster.bounds().unwrap();
                let average = AreaAverage::new_affine(&device, raster).await.unwrap();
                let tile = texture(
                    6,
                    4,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                for y in (0..bounds.height).step_by(4) {
                    for x in (0..bounds.width).step_by(6) {
                        let data: Vec<[f32; 4]> = (0..4)
                            .flat_map(|dy| {
                                (0..6).map(move |dx| {
                                    let rgb = pixel(bounds.x + x + dx, bounds.y + y + dy);
                                    [rgb[0], rgb[1], rgb[2], 1.]
                                })
                            })
                            .collect();
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &tile,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            bytemuck::cast_slice(&data),
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(6 * 16),
                                rows_per_image: None,
                            },
                            tile.size(),
                        );
                        let mut encoder = device.create_command_encoder(&Default::default());
                        average
                            .accumulate(&device, &mut encoder, &tile, [x, y])
                            .unwrap();
                        queue.submit([encoder.finish()]);
                        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    }
                }
                let result = texture(
                    output[0],
                    output[1],
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                );
                let stride = (output[0] * 16).div_ceil(256) * 256;
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(stride * output[1]),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                average.finish(&device, &mut encoder, &result).unwrap();
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &result,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(stride),
                            rows_per_image: None,
                        },
                    },
                    result.size(),
                );
                queue.submit([encoder.finish()]);
                let (send, recv) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                recv.recv().unwrap().unwrap();
                let data = buffer.slice(..).get_mapped_range().unwrap();
                for y in 0..output[1] {
                    for x in 0..output[0] {
                        let coverage = raster.footprint([x, y]).unwrap();
                        let mut expected = [0.; 3];
                        for ([sx, sy], weight) in coverage.pixels(area).unwrap() {
                            for (dst, value) in expected.iter_mut().zip(pixel(sx, sy)) {
                                *dst += f64::from(value) * weight / coverage.area();
                            }
                        }
                        let offset = (y * stride + x * 16) as usize;
                        let actual: &[f32] = bytemuck::cast_slice(&data[offset..offset + 16]);
                        for c in 0..3 {
                            assert!(
                                (f64::from(actual[c]) - expected[c]).abs() < 2e-5,
                                "angle={angle} output={output:?} pixel={x},{y} c={c}: {} vs {}",
                                actual[c],
                                expected[c]
                            );
                        }
                    }
                }
            }
        }
    });
}
