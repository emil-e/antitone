#![cfg(all(feature = "ui", feature = "render", feature = "ocio-reference"))]

#[path = "support/render_fixture.rs"]
mod render_fixture;
use antitone::adjustment_layers::{
    AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
};
#[path = "support/calculation.rs"]
mod calculation;
use antitone::{
    color::{Balance, ap1_to_ap0},
    flat_field::FlatField,
    gpu::{flat::GpuFlatField, parameters::ProcessingParameters, source::SourceGeometry},
    processing::{Calibration, DEFAULT_GAMMA, InputSettings, Pipeline, Response, RollSettings},
    rendering::{CpuRenderer, OutputTarget},
    viewer::GpuImage,
};
use calculation::ImageCalculations;
use glam::{DMat3, DVec3};

#[test]
fn corrected_viewer_shares_the_field_and_matches_both_cpu_output_targets() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) =
            antitone::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap();
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
        };
        let calibration = (base).validate().unwrap();
        let mut cases = vec![(
            calculation::inspection_with_field(ProcessingParameters::source(), Some(gpu.clone())),
            scans.iter().map(|p| matrix * p).collect::<Vec<_>>(),
        )];
        for response in [
            Response::film(),
            Response {
                toe: 3.5,
                shoulder: 0.01,
            },
        ] {
            let input = InputSettings::Negative {
                dmin: Some(calibration.dmin),
                dmax: Some(calibration.dmax),
                response,
                gamma: DEFAULT_GAMMA,
                balance: Balance::default(),
            };
            let roll = RollSettings {
                exposure: -2.,
                ..RollSettings::default()
            };
            let pipeline = Pipeline::new(input, roll, matrix).unwrap();
            cases.push((
                calculation::photograph_with_field(
                    &device.limits(),
                    input,
                    roll,
                    AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                    matrix,
                    [12, 12, 8640, 5760].into(),
                    Some(gpu.clone()),
                )
                .unwrap(),
                scans
                    .iter()
                    .map(|p| pipeline.apply(*p).unwrap().rgb)
                    .collect(),
            ));
        }
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let image = GpuImage::new(
                &device,
                &queue,
                [8, 8],
                &pixels,
                matrix,
                Some(geometry),
                target,
            )
            .await
            .unwrap();
            assert_eq!(image.processing_byte_len(), (64 * 8, 64 * 16));
            for (parameters, working) in &cases {
                let mut encoder = device.create_command_encoder(&Default::default());
                image
                    .encode_authored(&device, &mut encoder, parameters)
                    .unwrap();
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
        let (device, queue) =
            antitone::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap();
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
        };
        let input = InputSettings::Negative {
            dmin: Some(calibration.dmin),
            dmax: Some(calibration.dmax),
            response: Response::film(),
            gamma: 0.65,
            balance: Balance {
                warmth: 0.2,
                tint: -0.1,
            },
        };
        let roll = RollSettings {
            exposure: -2.,
            ..RollSettings::default()
        };
        let photo = AdjustmentLayerSettings {
            wb: Balance {
                warmth: -0.1,
                tint: 0.2,
            },
            exposure: 0.5,
            contrast: 1.2,
            saturation: 1.3,
            ..AdjustmentLayerSettings::default()
        };
        let pipeline = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            antitone::adjustment_layers::AdjustmentLayer::global(photo),
        ])
        .and_then(|layers| {
            antitone::adjustment_layers::AdjustmentLayerPipeline::new(input, roll, layers, matrix)
        })
        .unwrap();
        let positive: Vec<_> = scans
            .iter()
            .map(|p| pipeline.apply(*p, [0, 0]).unwrap().rgb)
            .collect();
        let films: Vec<_> = [
            Response::film(),
            Response {
                toe: 3.875,
                shoulder: 0.02,
            },
        ]
        .map(|response| {
            let settings = InputSettings::Negative {
                dmin: Some(calibration.dmin),
                dmax: Some(calibration.dmax),
                response,
                gamma: 0.65,
                balance: Balance {
                    warmth: 0.2,
                    tint: -0.1,
                },
            };
            let pipeline = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                antitone::adjustment_layers::AdjustmentLayer::global(photo),
            ])
            .and_then(|layers| {
                antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                    settings, roll, layers, matrix,
                )
            })
            .unwrap();
            let values: Vec<_> = scans
                .iter()
                .map(|s| pipeline.apply(*s, [0, 0]).unwrap().rgb)
                .collect();
            (
                AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(photo)])
                    .and_then(|snapshot| {
                        calculation::photograph(
                            &device.limits(),
                            settings.negative_input().unwrap(),
                            roll,
                            snapshot,
                            matrix,
                            [0, 0, 8192, 8192].into(),
                        )
                    })
                    .unwrap(),
                values,
            )
        })
        .into();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let image = GpuImage::new(&device, &queue, [4, 2], &pixels, matrix, None, target)
                .await
                .unwrap();
            assert_eq!(
                image.processing_byte_len(),
                (pixels.len() as u64 * 8, pixels.len() as u64 * 16)
            );
            assert!(
                GpuImage::new(&device, &queue, [4, 2], &pixels[..7], matrix, None, target)
                    .await
                    .is_err()
            );
            let presentation = antitone::viewer::Presentation::new(&device).await.unwrap();
            let rendered = GpuImage::new_rendered(&device, 4, 2, target).await.unwrap();
            assert_eq!(rendered.processing_byte_len(), (0, 0));
            assert!(
                rendered
                    .inspection(ProcessingParameters::source(), None)
                    .is_err()
            );
            for (parameters, working) in [
                (
                    calculation::inspection(ProcessingParameters::source()),
                    &working,
                ),
                (
                    AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(photo)])
                        .and_then(|snapshot| {
                            calculation::photograph(
                                &device.limits(),
                                input.negative_input().unwrap(),
                                roll,
                                snapshot,
                                matrix,
                                [0, 0, 8192, 8192].into(),
                            )
                        })
                        .unwrap(),
                    &positive,
                ),
                (films[0].0.clone(), &films[0].1),
                (films[1].0.clone(), &films[1].1),
            ] {
                let mut encoder = device.create_command_encoder(&Default::default());
                image
                    .encode_authored(&device, &mut encoder, &parameters)
                    .unwrap();
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
                    let draw = presentation.bind(&image.linear, target, uv).unwrap();
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
                        draw.paint(&mut pass.forget_lifetime());
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
fn oriented_presentation_matches_exact_source_pixel_mapping() {
    use antitone::crop::Crop;
    use antitone::geometry::{
        Composition, Flips, FramePoint, PhotoOrientation, QuarterTurns, RollOrientation,
        SourceRegion,
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) =
            antitone::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap();
        let presentation = antitone::viewer::Presentation::new(&device).await.unwrap();
        let area = SourceRegion {
            x: 12,
            y: 7,
            width: 5,
            height: 3,
        };
        // Distinct target-linear texels, including signed and extended-range values.
        let linear: Vec<[f32; 4]> = (0..15)
            .map(|i| {
                [
                    -0.05 + i as f32 * 0.08,
                    0.12 + i as f32 * 0.03,
                    0.7 - i as f32 * 0.04,
                    1.,
                ]
            })
            .collect();
        let size = wgpu::Extent3d {
            width: area.width,
            height: area.height,
            depth_or_array_layers: 1,
        };
        let image = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("authored target-linear pixels"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            image.as_image_copy(),
            bytemuck::cast_slice(&linear),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(area.width * 16),
                rows_per_image: Some(area.height),
            },
            size,
        );
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let crop = Some(Crop::from_region(SourceRegion {
                x: 13,
                y: 8,
                width: 3,
                height: 2,
            }));
            for (crop, margin) in [(None, 0), (crop, 0), (crop, 2)] {
                for roll_turns in [
                    QuarterTurns::Zero,
                    QuarterTurns::One,
                    QuarterTurns::Two,
                    QuarterTurns::Three,
                ] {
                    for roll_horizontal in [false, true] {
                        for roll_vertical in [false, true] {
                            for turns in [
                                QuarterTurns::Zero,
                                QuarterTurns::One,
                                QuarterTurns::Two,
                                QuarterTurns::Three,
                            ] {
                                for photo_horizontal in [false, true] {
                                    for photo_vertical in [false, true] {
                                        let composition = Composition::with_crop(
                                            area,
                                            RollOrientation {
                                                turns: roll_turns,
                                                horizontal: roll_horizontal,
                                                vertical: roll_vertical,
                                            },
                                            PhotoOrientation {
                                                turns,
                                                flips: Flips {
                                                    horizontal: photo_horizontal,
                                                    vertical: photo_vertical,
                                                },
                                            },
                                            crop,
                                        )
                                        .unwrap();
                                        let [frame_width, frame_height] = composition.size();
                                        let [width, height] =
                                            [frame_width + margin * 2, frame_height + margin * 2];
                                        let texture =
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
                                                format: wgpu::TextureFormat::Rgba16Float,
                                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                                    | wgpu::TextureUsages::COPY_SRC,
                                                view_formats: &[],
                                            });
                                        let view = texture.create_view(&Default::default());
                                        let mapping = composition.texture_mapping(
                                            FramePoint([-f64::from(margin), -f64::from(margin)]),
                                            FramePoint([
                                                f64::from(frame_width + margin),
                                                f64::from(frame_height + margin),
                                            ]),
                                            area,
                                        );
                                        let draw =
                                            presentation.bind(&image, target, mapping).unwrap();
                                        let mut encoder =
                                            device.create_command_encoder(&Default::default());
                                        {
                                            let pass = encoder.begin_render_pass(
                                                &wgpu::RenderPassDescriptor {
                                                    label: None,
                                                    color_attachments: &[Some(
                                                        wgpu::RenderPassColorAttachment {
                                                            view: &view,
                                                            depth_slice: None,
                                                            resolve_target: None,
                                                            ops: wgpu::Operations {
                                                                load: wgpu::LoadOp::Clear(
                                                                    wgpu::Color::BLACK,
                                                                ),
                                                                store: wgpu::StoreOp::Store,
                                                            },
                                                        },
                                                    )],
                                                    ..Default::default()
                                                },
                                            );
                                            draw.paint(&mut pass.forget_lifetime());
                                        }
                                        queue.submit([encoder.finish()]);
                                        let actual = read(&device, &queue, &texture).await;
                                        for y in 0..height {
                                            for x in 0..width {
                                                let point = composition
                                                    .frame_to_source(FramePoint([
                                                        f64::from(x) - f64::from(margin) + 0.5,
                                                        f64::from(y) - f64::from(margin) + 0.5,
                                                    ]))
                                                    .0;
                                                let sample = if point[0] >= f64::from(area.x)
                                                    && point[0] < f64::from(area.x + area.width)
                                                    && point[1] >= f64::from(area.y)
                                                    && point[1] < f64::from(area.y + area.height)
                                                {
                                                    let [sx, sy] = point.map(|v| v.floor() as u32);
                                                    linear[((sy - area.y) * area.width + sx
                                                        - area.x)
                                                        as usize]
                                                } else {
                                                    [0., 0., 0., 1.]
                                                };
                                                let expected = target.linear_to_srgb()
                                                    * DVec3::new(
                                                        f64::from(sample[0]),
                                                        f64::from(sample[1]),
                                                        f64::from(sample[2]),
                                                    );
                                                for channel in 0..3 {
                                                    let value = expected[channel];
                                                    let a = value.abs();
                                                    let encoded = value.signum()
                                                        * if a <= 0.0031308 {
                                                            12.92 * a
                                                        } else {
                                                            1.055 * a.powf(1. / 2.4) - 0.055
                                                        };
                                                    assert!(
                                                        (f64::from(
                                                            actual[(y * width + x) as usize]
                                                                [channel]
                                                        ) - encoded)
                                                            .abs()
                                                            < 0.0015,
                                                        "{composition:?} {x},{y}"
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
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
        let (device, queue) =
            antitone::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap();
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
            let image = GpuImage::new(&device, &queue, [321, 3], &pixels, matrix, None, target)
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

async fn read(
    device: &wgpu::Device,
    queue: &antitone::gpu::submission::Queue,
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
