#![cfg(all(feature = "ui", feature = "render", feature = "ocio-reference"))]

#[path = "support/render_fixture.rs"]
mod render_fixture;
use antitone::adjustment_layers::AdjustmentLayer;
use antitone::adjustment_layers::AdjustmentLayerCoverage;
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::adjustment_layers::AdjustmentLayerSnapshot;
use antitone::color::Balance;
use antitone::flat_field::FlatField;
use antitone::frame_raster::FrameRaster;
use antitone::geometry::Composition;
use antitone::geometry::SourcePoint;
use antitone::geometry::SourceRegion;
use antitone::gpu::flat::GpuFlatField;
#[path = "support/calculation.rs"]
mod calculation;
use antitone::gpu::parameters::ProcessingParameters;
use antitone::gpu::source::SourceGeometry;
use antitone::gpu_scopes::GpuScopes;
use antitone::processing::Calibration;
use antitone::processing::Pipeline;
use antitone::processing::RollSettings;
use antitone::rendering::CpuRenderer;
use antitone::rendering::OutputTarget;
use antitone::scopes;
use antitone::scopes::Plot;
use antitone::scopes::Settings;
use antitone::viewer::GpuImage;
use calculation::{Calculation, ImageCalculations, ScopeCalculations};
use glam::{DMat3, DVec3};
use std::sync::Arc;

#[path = "support/photographic_reference.rs"]
mod reference;

const AREA: SourceRegion = SourceRegion {
    x: 120,
    y: 73,
    width: 23,
    height: 19,
};

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.8, 0.7, 0.9),
        dmax: DVec3::new(0.003, 0.002, 0.001),
    }
}

fn layers() -> Vec<AdjustmentLayer> {
    vec![
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([122., 74.]),
                end: SourcePoint([138., 89.]),
            },
            photo: AdjustmentLayerSettings {
                exposure: 0.7,
                contrast: 1.2,
                contrast_colour_mix: 0.3,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.8,
        },
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Radial {
                center: SourcePoint([131., 82.]),
                radii: [10., 7.],
                angle: 0.4,
                feather: 0.65,
            },
            photo: AdjustmentLayerSettings {
                exposure: -0.3,
                wb: Balance {
                    warmth: 0.13,
                    tint: -0.07,
                },
                saturation: 1.4,
                vibrance: 0.6,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.9,
        },
    ]
}

fn input(
    device: &wgpu::Device,
    cal: Calibration,
    layers: Vec<AdjustmentLayer>,
    field: Option<Arc<GpuFlatField>>,
) -> Calculation {
    calculation::photograph_with_field(
        &device.limits(),
        cal,
        RollSettings::default(),
        AdjustmentLayerSnapshot::new(
            std::iter::once(antitone::adjustment_layers::AdjustmentLayer::global(
                AdjustmentLayerSettings::default(),
            ))
            .chain(layers)
            .collect(),
        )
        .unwrap(),
        DMat3::IDENTITY,
        AREA,
        field,
    )
    .unwrap()
}

async fn device() -> (wgpu::Device, antitone::gpu::submission::Queue) {
    let instance = antitone::gpu::instance();
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    antitone::gpu::submission::request_device(&adapter, &Default::default())
        .await
        .unwrap()
}

fn codes([x, y]: [u32; 2]) -> [u16; 4] {
    [
        600 + ((x * 13 + y * 7) % 31) as u16 * 37,
        700 + ((x * 7 + y * 11) % 23) as u16 * 43,
        800 + ((x * 19 + y * 3) % 41) as u16 * 29,
        65535,
    ]
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

fn local_reference(mut rgb: DVec3, pixel: [u32; 2]) -> DVec3 {
    let [x, y] = pixel.map(|v| f64::from(v) + 0.5);
    let linear = 0.8 * smooth(((x - 122.) * 16. + (y - 74.) * 15.) / (16. * 16. + 15. * 15.));
    rgb *= (0.7 * linear).exp2();
    let contrast = 1. + 0.2 * linear;
    rgb = reference::contrast_mix(rgb, contrast, 0.18, 0.3).unwrap();
    let (sin, cos) = 0.4_f64.sin_cos();
    let dx = x - 131.;
    let dy = y - 82.;
    let radius = ((dx * cos + dy * sin) / 10.).hypot((-dx * sin + dy * cos) / 7.);
    let radial = 0.9 * smooth((1. - radius) / 0.65);
    rgb *= (-0.3 * radial).exp2();
    let warmth = 0.13 * radial / 2_f64.sqrt();
    let tint = -0.07 * radial / 6_f64.sqrt();
    let gains = DVec3::new(warmth + tint, -2. * tint, -warmth + tint).exp();
    let cones = antitone::color::BRADFORD * antitone::color::AP1_TO_XYZ;
    rgb = (cones.inverse() * DMat3::from_diagonal(gains) * cones) * rgb;
    let saturation = 1. + 0.4 * radial;
    reference::saturation(rgb, saturation, 0.6 * radial).unwrap()
}

async fn image(
    renderer: &Arc<crate::render_fixture::RenderFixture>,
    geometry: SourceGeometry,
    size: [u32; 2],
    target: OutputTarget,
) -> GpuImage {
    let pixels: Vec<_> = (0..size[0] * size[1])
        .map(|i| codes(geometry.pixel(i).unwrap()))
        .collect();
    GpuImage::new_shared(
        renderer.evaluation().clone(),
        renderer.looks().clone(),
        renderer.drt(target),
        size,
        &pixels,
        DMat3::IDENTITY,
        Some(geometry),
    )
    .await
    .unwrap()
}

async fn read_image(
    device: &wgpu::Device,
    queue: &antitone::gpu::submission::Queue,
    image: &GpuImage,
) -> Vec<[f32; 4]> {
    let texture = &image.linear;
    let row = (texture.width() * 16).div_ceil(256) * 256;
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
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = buffer.get_mapped_range(..).unwrap();
    let pixels = mapped
        .chunks_exact(row as usize)
        .flat_map(|line| {
            bytemuck::cast_slice::<u8, [f32; 4]>(&line[..texture.width() as usize * 16]).to_vec()
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    pixels
}

fn read_buffer(
    device: &wgpu::Device,
    queue: &antitone::gpu::submission::Queue,
    source: &wgpu::Buffer,
) -> Vec<u32> {
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
    staging.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let result = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    staging.unmap();
    result
}

#[test]
fn preparation_uses_grade_content_and_rejects_candidates_without_changing_the_cache() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = crate::render_fixture::RenderFixture::new(&device, &queue)
            .await
            .unwrap();
        let geometry =
            SourceGeometry::new([AREA.x, AREA.y, AREA.width, AREA.height], [7, 5]).unwrap();
        let image = Arc::new(image(&renderer, geometry, [7, 5], OutputTarget::Srgb).await);
        let accepted = input(&device, calibration(), layers(), None);
        let duplicate = input(&device, calibration(), layers(), None);
        let accepted_definition = accepted(&image).unwrap();
        assert!(
            accepted_definition.same_definition(&duplicate(&image).unwrap()),
            "allocation identity is not calculation identity"
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            image
                .prepare_authored(&device, &mut encoder, &accepted)
                .unwrap()
        );
        assert!(
            !image
                .prepare_authored(&device, &mut encoder, &duplicate)
                .unwrap()
        );
        for changed in [
            {
                let mut a = layers();
                a.reverse();
                a
            },
            {
                let mut a = layers();
                a[0].enabled = false;
                a
            },
            {
                let mut a = layers();
                a[0].strength = 0.5;
                a
            },
            {
                let mut a = layers();
                a[0].invert = true;
                a
            },
            {
                let mut a = layers();
                a[0].mask = AdjustmentLayerCoverage::Linear {
                    start: SourcePoint([120., 73.]),
                    end: SourcePoint([140., 90.]),
                };
                a
            },
        ] {
            let proposed = input(&device, calibration(), changed, None);
            assert!(!accepted_definition.same_definition(&proposed(&image).unwrap()));
            assert!(
                image
                    .prepare_authored(&device, &mut encoder, &proposed)
                    .unwrap()
            );
            assert!(
                image
                    .prepare_authored(&device, &mut encoder, &accepted)
                    .unwrap()
            );
        }
        let mut changed_calibration = calibration();
        changed_calibration.dmin.x *= 0.9;
        assert!(!accepted_definition.same_definition(
            &input(&device, changed_calibration, layers(), None)(&image).unwrap(),
        ));
        for (roll, photo, area) in [
            (
                RollSettings {
                    exposure: 0.5,
                    ..RollSettings::default()
                },
                AdjustmentLayerSettings::default(),
                AREA,
            ),
            (
                RollSettings::default(),
                AdjustmentLayerSettings {
                    exposure: 0.5,
                    ..Default::default()
                },
                AREA,
            ),
            (
                RollSettings::default(),
                AdjustmentLayerSettings::default(),
                SourceRegion {
                    x: AREA.x - 1,
                    width: AREA.width + 1,
                    ..AREA
                },
            ),
        ] {
            let changed: Calculation = calculation::photograph(
                &device.limits(),
                calibration(),
                roll,
                AdjustmentLayerSnapshot::new(
                    std::iter::once(antitone::adjustment_layers::AdjustmentLayer::global(photo))
                        .chain(layers())
                        .collect(),
                )
                .unwrap(),
                DMat3::IDENTITY,
                area,
            )
            .unwrap();
            assert!(!accepted_definition.same_definition(&changed(&image).unwrap()));
        }
        let second = self::image(&renderer, geometry, [7, 5], OutputTarget::Srgb).await;
        assert!(
            second
                .prepare_calculation(
                    &device,
                    &mut encoder,
                    &accepted_definition,
                    &Default::default()
                )
                .unwrap(),
            "an immutable captured source can render into another compatible target"
        );
        assert!(
            second
                .prepare_authored(&device, &mut encoder, &accepted)
                .unwrap(),
            "images share resources, not preparation state"
        );
        let unlocated = GpuImage::new_shared(
            renderer.evaluation().clone(),
            renderer.looks().clone(),
            renderer.drt(OutputTarget::Srgb),
            [7, 5],
            &[[0; 4]; 35],
            DMat3::IDENTITY,
            None,
        )
        .await
        .unwrap();
        assert!(
            unlocated
                .prepare_authored(&device, &mut encoder, &accepted)
                .is_err()
        );
        let different_basis: Calculation = calculation::photograph(
            &device.limits(),
            antitone::processing::InputSettings::Digital,
            RollSettings::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            DMat3::from_diagonal(DVec3::splat(2.)),
            AREA,
        )
        .unwrap();
        assert!(
            image
                .prepare_authored(&device, &mut encoder, &different_basis)
                .unwrap()
        );
        queue.submit([encoder.finish()]);
        let working: Vec<_> = (0..35)
            .map(|index| {
                let p = codes(geometry.pixel(index).unwrap());
                DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535. * 2.
            })
            .collect();
        let expected = CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&working)
            .unwrap();
        let actual = read_image(&device, &queue, &image).await;
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() < 2e-5,
                    "operation-owned matrix must affect unchanged camera samples: {actual:?} vs {expected:?}"
                );
            }
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            image
                .prepare_authored(&device, &mut encoder, &accepted)
                .unwrap()
        );
        image.encode(
            &device,
            &mut encoder,
            ProcessingParameters::source_inspection(1.).unwrap(),
        );
        assert!(
            image
                .prepare_authored(&device, &mut encoder, &accepted)
                .unwrap(),
            "explicit source bypass invalidates previous output identity"
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    });
}

#[test]
fn viewer_callbacks_apply_locals_at_proxy_source_centres_with_and_without_correction() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = crate::render_fixture::RenderFixture::new(&device, &queue)
            .await
            .unwrap();
        let field = Arc::new(
            FlatField::from_nodes(
                [AREA.x, AREA.y, AREA.width, AREA.height],
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
        let gpu_field = Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
        let presentation = antitone::viewer::Presentation::new(&device).await.unwrap();
        let geometry =
            SourceGeometry::new([AREA.x, AREA.y, AREA.width, AREA.height], [7, 5]).unwrap();
        for corrected in [false, true] {
            let cal = if corrected {
                (calibration()).validate().unwrap()
            } else {
                calibration()
            };
            let processing = input(&device, cal, layers(), corrected.then(|| gpu_field.clone()));
            let global = Pipeline::new(cal, RollSettings::default(), DMat3::IDENTITY).unwrap();
            let working: Vec<_> = (0..35)
                .map(|i| {
                    let pixel = geometry.pixel(i).unwrap();
                    let p = codes(pixel);
                    let gain = if corrected {
                        field.gain(pixel.map(|v| f64::from(v) + 0.5)).unwrap()
                    } else {
                        DVec3::ONE
                    };
                    local_reference(
                        global
                            .apply(
                                DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535. * gain,
                            )
                            .unwrap()
                            .rgb,
                        pixel,
                    )
                })
                .collect();
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let image = Arc::new(image(&renderer, geometry, [7, 5], target).await);
                let paint = calculation::image_paint(
                    &presentation,
                    image.clone(),
                    Some(processing.clone()),
                    antitone::geometry::TextureMapping::IDENTITY,
                )
                .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                paint.prepare_frame(&device, &mut encoder);
                assert!(
                    !image
                        .prepare_authored(&device, &mut encoder, &processing)
                        .unwrap()
                );
                queue.submit([encoder.finish()]);
                let actual = read_image(&device, &queue, &image).await;
                let cpu = CpuRenderer::new(target).unwrap();
                let expected = cpu.render(&working).unwrap();
                let globals: Vec<_> = (0..35)
                    .map(|i| {
                        let point = geometry.pixel(i).unwrap();
                        let p = codes(point);
                        let gain = if corrected {
                            field.gain(point.map(|v| f64::from(v) + 0.5)).unwrap()
                        } else {
                            DVec3::ONE
                        };
                        global
                            .apply(
                                DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535. * gain,
                            )
                            .unwrap()
                            .rgb
                    })
                    .collect();
                let without_locals = cpu.render(&globals).unwrap();
                assert!(
                    expected
                        .iter()
                        .zip(without_locals)
                        .any(|(local, global)| (*local - global).abs().max_element() > 2e-5),
                    "fixture must distinguish layers from a global-only render"
                );
                for (a, b) in actual.iter().zip(expected) {
                    for c in 0..3 {
                        assert!(
                            (a[c] - b[c]).abs() < 2e-5,
                            "corrected={corrected} target={target:?}: {a:?} vs {b:?}"
                        );
                    }
                }
                let bypass = calculation::inspection_with_field(
                    ProcessingParameters::source_inspection(1.).unwrap(),
                    corrected.then(|| gpu_field.clone()),
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                image
                    .encode_authored(&device, &mut encoder, &bypass)
                    .unwrap();
                queue.submit([encoder.finish()]);
                let source: Vec<_> = (0..35)
                    .map(|i| {
                        let point = geometry.pixel(i).unwrap();
                        let p = codes(point);
                        let gain = if corrected {
                            field.gain(point.map(|v| f64::from(v) + 0.5)).unwrap()
                        } else {
                            DVec3::ONE
                        };
                        DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535. * gain * 2.
                    })
                    .collect();
                let expected = CpuRenderer::new(target).unwrap().render(&source).unwrap();
                for (a, b) in read_image(&device, &queue, &image)
                    .await
                    .iter()
                    .zip(expected)
                {
                    for c in 0..3 {
                        assert!((a[c] - b[c]).abs() < 2e-5, "source bypass: {a:?} vs {b:?}");
                    }
                }
            }
        }
    });
}

#[test]
fn scopes_share_local_preparation_and_do_not_label_held_readbacks_as_the_new_grade() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = crate::render_fixture::RenderFixture::new(&device, &queue)
            .await
            .unwrap();
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let geometry =
            SourceGeometry::new([AREA.x, AREA.y, AREA.width, AREA.height], [7, 5]).unwrap();
        let image = Arc::new(image(&renderer, geometry, [7, 5], OutputTarget::Srgb).await);
        let mut layers = layers();
        layers[0].photo.contrast = 1.6;
        layers[0].photo.exposure = 2.;
        layers[1].photo.exposure = -2.;
        layers[1].photo.wb.warmth = 0.5;
        let first = input(&device, calibration(), layers.clone(), None);
        let mut reversed = layers.clone();
        reversed.reverse();
        let latest = input(&device, calibration(), reversed, None);
        let frame = FrameRaster::new(
            Composition::new(AREA, Default::default(), Default::default()).unwrap(),
            antitone::geometry::FramePoint([0., 0.]),
            antitone::geometry::FramePoint([23., 19.]),
            [7, 5],
        )
        .unwrap();
        let settings = Settings {
            target: OutputTarget::Srgb,
            plot: Plot::Rgb,
            amplification: 1,
        };
        let context = egui::Context::default();
        let paint = |input: Calculation| {
            calculation::scope_paint(
                gpu.clone(),
                image.clone(),
                input,
                frame,
                settings,
                context.clone(),
            )
            .unwrap()
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(first.clone()).prepare_frame(&device, &mut encoder);
        assert!(
            !image
                .prepare_authored(&device, &mut encoder, &first)
                .unwrap()
        );
        paint(latest.clone()).prepare_frame(&device, &mut encoder);
        assert_eq!(gpu.reduction_count(), 1, "one readback remains outstanding");
        assert!(
            gpu.status_authored(&image, &latest, frame, settings)
                .is_none()
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            gpu.status_authored(&image, &latest, frame, settings)
                .is_none(),
            "old completion is not current"
        );
        let (expected_bins, _, _) =
            scopes::reference([7, 5], &read_image(&device, &queue, &image).await, settings)
                .unwrap();
        assert!(
            read_buffer(&device, &queue, &gpu.bins) != expected_bins,
            "reordering must distinguish these fixtures' scope data"
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(latest.clone()).prepare_frame(&device, &mut encoder);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(gpu.reduction_count(), 2);
        for (i, (actual, expected)) in read_buffer(&device, &queue, &gpu.bins)
            .iter()
            .zip(&expected_bins)
            .enumerate()
        {
            assert_eq!(actual, expected, "scope bin {i}");
        }
        assert_eq!(
            gpu.status_authored(&image, &latest, frame, settings)
                .unwrap()
                .unwrap()
                .valid,
            35
        );
        let (_, _, counts) =
            scopes::reference([7, 5], &read_image(&device, &queue, &image).await, settings)
                .unwrap();
        assert_eq!(
            gpu.status_authored(&image, &latest, frame, settings)
                .unwrap()
                .unwrap(),
            counts
        );
        let duplicate = {
            let mut a = layers.clone();
            a.reverse();
            input(&device, calibration(), a, None)
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(duplicate).prepare_frame(&device, &mut encoder);
        assert_eq!(
            gpu.reduction_count(),
            2,
            "identical content reuses bins across allocations"
        );
        gpu.invalidate();
        queue.submit([encoder.finish()]);
        assert!(
            gpu.status_authored(&image, &latest, frame, settings)
                .is_none()
        );
    });
}
