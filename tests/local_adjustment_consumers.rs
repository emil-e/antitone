#![cfg(all(feature = "ui", feature = "render", feature = "ocio-reference"))]
use antitone::{
    color::Balance,
    flat_field::FlatField,
    frame_raster::FrameRaster,
    geometry::{Composition, SourcePoint, SourceRegion},
    gpu::{GpuFlatField, GpuPhotoGrade, ProcessingInput, ProcessingParameters, SourceGeometry},
    gpu_scopes::{GpuScopes, ScopePaint},
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot},
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    rendering::{CpuRenderer, OutputTarget},
    scopes::{self, Plot, Settings},
    viewer::{GpuImage, ImagePaint},
};
use egui_wgpu::CallbackTrait;
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
        input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
    }
}

fn adjustments() -> Vec<LocalAdjustment> {
    vec![
        LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([122., 74.]),
                end: SourcePoint([138., 89.]),
            },
            photo: PhotoSettings {
                exposure: 0.7,
                contrast: 1.2,
                contrast_colour_mix: 0.3,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.8,
        },
        LocalAdjustment {
            mask: GradientMask::Radial {
                center: SourcePoint([131., 82.]),
                radii: [10., 7.],
                angle: 0.4,
                feather: 0.65,
            },
            photo: PhotoSettings {
                exposure: -0.3,
                wb: Balance {
                    warmth: 0.13,
                    tint: -0.07,
                },
                saturation: 1.4,
                saturation_density: 0.6,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.9,
        },
    ]
}

fn input(device: &wgpu::Device, cal: Calibration, locals: Vec<LocalAdjustment>) -> ProcessingInput {
    Arc::new(
        GpuPhotoGrade::new(
            device,
            cal,
            RollSettings::default(),
            PhotoGradeSnapshot::new(PhotoSettings::default(), locals).unwrap(),
            DMat3::IDENTITY,
            AREA,
        )
        .unwrap(),
    )
    .into()
}

async fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    adapter.request_device(&Default::default()).await.unwrap()
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
    rgb = reference::blend(
        reference::contrast(rgb, contrast, 0.18, reference::Contrast::Rgb).unwrap(),
        reference::contrast(rgb, contrast, 0.18, reference::Contrast::BlackOffsetRatios).unwrap(),
        0.3,
    )
    .unwrap();
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
    reference::blend(
        reference::saturation(rgb, saturation, reference::Saturation::ConstantY).unwrap(),
        reference::saturation(rgb, saturation, reference::Saturation::Maximum).unwrap(),
        0.6,
    )
    .unwrap()
}

async fn image(
    renderer: &Arc<antitone::renderer::GpuRenderer>,
    device: &wgpu::Device,
    geometry: SourceGeometry,
    size: [u32; 2],
    target: OutputTarget,
    field: Option<Arc<GpuFlatField>>,
) -> GpuImage {
    let pixels: Vec<_> = (0..size[0] * size[1])
        .map(|i| codes(geometry.pixel(i).unwrap()))
        .collect();
    let mut image = GpuImage::new_shared(
        renderer,
        device,
        size[0],
        size[1],
        &pixels,
        DMat3::IDENTITY,
        target,
    )
    .await
    .unwrap();
    image.set_flat_field(field, geometry).unwrap();
    image
}

async fn read_image(device: &wgpu::Device, queue: &wgpu::Queue, image: &GpuImage) -> Vec<[f32; 4]> {
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

fn read_buffer(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Vec<u32> {
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

fn screen() -> egui_wgpu::ScreenDescriptor {
    egui_wgpu::ScreenDescriptor {
        size_in_pixels: [600, 400],
        pixels_per_point: 2.,
    }
}

#[test]
fn preparation_uses_grade_content_and_rejects_candidates_without_changing_the_cache() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = antitone::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let geometry =
            SourceGeometry::new([AREA.x, AREA.y, AREA.width, AREA.height], [7, 5]).unwrap();
        let image = Arc::new(
            image(
                &renderer,
                &device,
                geometry,
                [7, 5],
                OutputTarget::Srgb,
                None,
            )
            .await,
        );
        let accepted = input(&device, calibration(), adjustments());
        let duplicate = input(&device, calibration(), adjustments());
        assert_eq!(
            accepted.signature(),
            duplicate.signature(),
            "allocation identity is not processing identity"
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &accepted)
                .unwrap()
        );
        assert!(
            !image
                .prepare_processing(&device, &mut encoder, &duplicate)
                .unwrap()
        );
        for changed in [
            {
                let mut a = adjustments();
                a.reverse();
                a
            },
            {
                let mut a = adjustments();
                a[0].enabled = false;
                a
            },
            {
                let mut a = adjustments();
                a[0].strength = 0.5;
                a
            },
            {
                let mut a = adjustments();
                a[0].invert = true;
                a
            },
            {
                let mut a = adjustments();
                a[0].mask = GradientMask::Linear {
                    start: SourcePoint([120., 73.]),
                    end: SourcePoint([140., 90.]),
                };
                a
            },
        ] {
            let proposed = input(&device, calibration(), changed);
            assert_ne!(accepted.signature(), proposed.signature());
            assert!(
                image
                    .prepare_processing(&device, &mut encoder, &proposed)
                    .unwrap()
            );
            assert!(
                image
                    .prepare_processing(&device, &mut encoder, &accepted)
                    .unwrap()
            );
        }
        let mut changed_calibration = calibration();
        changed_calibration.dmin.x *= 0.9;
        assert_ne!(
            accepted.signature(),
            input(&device, changed_calibration, adjustments()).signature()
        );
        for (roll, photo, area) in [
            (
                RollSettings {
                    exposure: 0.5,
                    ..RollSettings::default()
                },
                PhotoSettings::default(),
                AREA,
            ),
            (
                RollSettings::default(),
                PhotoSettings {
                    exposure: 0.5,
                    ..Default::default()
                },
                AREA,
            ),
            (
                RollSettings::default(),
                PhotoSettings::default(),
                SourceRegion {
                    x: AREA.x - 1,
                    width: AREA.width + 1,
                    ..AREA
                },
            ),
        ] {
            let changed: ProcessingInput = Arc::new(
                GpuPhotoGrade::new(
                    &device,
                    calibration(),
                    roll,
                    PhotoGradeSnapshot::new(photo, adjustments()).unwrap(),
                    DMat3::IDENTITY,
                    area,
                )
                .unwrap(),
            )
            .into();
            assert_ne!(accepted.signature(), changed.signature());
        }
        let second = self::image(
            &renderer,
            &device,
            geometry,
            [7, 5],
            OutputTarget::Srgb,
            None,
        )
        .await;
        assert!(
            second
                .prepare_processing(&device, &mut encoder, &accepted)
                .unwrap(),
            "images share resources, not preparation state"
        );
        let unlocated = GpuImage::new_shared(
            &renderer,
            &device,
            7,
            5,
            &[[0; 4]; 35],
            DMat3::IDENTITY,
            OutputTarget::Srgb,
        )
        .await
        .unwrap();
        assert!(
            unlocated
                .prepare_processing(&device, &mut encoder, &accepted)
                .is_err()
        );
        let wrong_basis: ProcessingInput = Arc::new(
            GpuPhotoGrade::new(
                &device,
                calibration(),
                RollSettings::default(),
                PhotoGradeSnapshot::new(PhotoSettings::default(), adjustments()).unwrap(),
                DMat3::from_diagonal(DVec3::splat(2.)),
                AREA,
            )
            .unwrap(),
        )
        .into();
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &wrong_basis)
                .is_err()
        );
        assert!(
            ImagePaint::with_processing(
                image.clone(),
                Some(wrong_basis),
                antitone::geometry::TextureMapping::IDENTITY
            )
            .is_err()
        );
        assert!(
            !image
                .prepare_processing(&device, &mut encoder, &accepted)
                .unwrap()
        );
        let revision = image.source_revision();
        image.encode(
            &device,
            &mut encoder,
            ProcessingParameters::source_inspection(1.).unwrap(),
        );
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &accepted)
                .unwrap(),
            "explicit source bypass invalidates previous output identity"
        );
        assert_eq!(
            image.source_revision(),
            revision,
            "grade edits do not redevelop the source"
        );
        image
            .write_source(&queue, &[[6000, 8000, 9000, 0]; 35])
            .unwrap();
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &accepted)
                .unwrap()
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    });
}

#[test]
fn viewer_callbacks_apply_locals_at_proxy_source_centres_with_and_without_correction() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = antitone::renderer::GpuRenderer::new(&device, &queue)
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
        let geometry =
            SourceGeometry::new([AREA.x, AREA.y, AREA.width, AREA.height], [7, 5]).unwrap();
        for corrected in [false, true] {
            let cal = if corrected {
                field.processing_calibration(calibration()).unwrap()
            } else {
                calibration()
            };
            let processing = input(&device, cal, adjustments());
            let global = Pipeline::new(
                cal,
                RollSettings::default(),
                PhotoSettings::default(),
                DMat3::IDENTITY,
            )
            .unwrap();
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
                let image = Arc::new(
                    image(
                        &renderer,
                        &device,
                        geometry,
                        [7, 5],
                        target,
                        corrected.then(|| gpu_field.clone()),
                    )
                    .await,
                );
                let paint = ImagePaint::with_processing(
                    image.clone(),
                    Some(processing.clone()),
                    antitone::geometry::TextureMapping::IDENTITY,
                )
                .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                paint.prepare(
                    &device,
                    &queue,
                    &screen(),
                    &mut encoder,
                    &mut Default::default(),
                );
                assert!(
                    !image
                        .prepare_processing(&device, &mut encoder, &processing)
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
                    "fixture must distinguish locals from a global-only render"
                );
                for (a, b) in actual.iter().zip(expected) {
                    for c in 0..3 {
                        assert!(
                            (a[c] - b[c]).abs() < 2e-5,
                            "corrected={corrected} target={target:?}: {a:?} vs {b:?}"
                        );
                    }
                }
                let bypass = ProcessingParameters::source_inspection(1.).unwrap().into();
                let mut encoder = device.create_command_encoder(&Default::default());
                image
                    .encode_processing(&device, &mut encoder, &bypass)
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
        let renderer = antitone::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let geometry =
            SourceGeometry::new([AREA.x, AREA.y, AREA.width, AREA.height], [7, 5]).unwrap();
        let image = Arc::new(
            image(
                &renderer,
                &device,
                geometry,
                [7, 5],
                OutputTarget::Srgb,
                None,
            )
            .await,
        );
        let mut locals = adjustments();
        locals[0].photo.contrast = 1.6;
        locals[0].photo.exposure = 2.;
        locals[1].photo.exposure = -2.;
        locals[1].photo.wb.warmth = 0.5;
        let first = input(&device, calibration(), locals.clone());
        let mut reversed = locals.clone();
        reversed.reverse();
        let latest = input(&device, calibration(), reversed);
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
        let paint = |input: ProcessingInput| {
            ScopePaint::with_processing(
                gpu.clone(),
                image.clone(),
                input,
                frame,
                settings,
                context.clone(),
            )
            .unwrap()
        };
        let mut resources = Default::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(first.clone()).prepare(&device, &queue, &screen(), &mut encoder, &mut resources);
        assert!(
            !image
                .prepare_processing(&device, &mut encoder, &first)
                .unwrap()
        );
        paint(latest.clone()).prepare(&device, &queue, &screen(), &mut encoder, &mut resources);
        assert_eq!(gpu.reduction_count(), 1, "one readback remains outstanding");
        assert!(
            gpu.status_processing(&image, &latest, frame, settings)
                .is_none()
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            gpu.status_processing(&image, &latest, frame, settings)
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
        paint(latest.clone()).prepare(&device, &queue, &screen(), &mut encoder, &mut resources);
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
            gpu.status_processing(&image, &latest, frame, settings)
                .unwrap()
                .unwrap()
                .valid,
            35
        );
        let (_, _, counts) =
            scopes::reference([7, 5], &read_image(&device, &queue, &image).await, settings)
                .unwrap();
        assert_eq!(
            gpu.status_processing(&image, &latest, frame, settings)
                .unwrap()
                .unwrap(),
            counts
        );
        let duplicate = {
            let mut a = locals.clone();
            a.reverse();
            input(&device, calibration(), a)
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        paint(duplicate).prepare(&device, &queue, &screen(), &mut encoder, &mut resources);
        assert_eq!(
            gpu.reduction_count(),
            2,
            "identical content reuses bins across allocations"
        );
        gpu.invalidate();
        queue.submit([encoder.finish()]);
        assert!(
            gpu.status_processing(&image, &latest, frame, settings)
                .is_none()
        );
    });
}

#[test]
fn reusable_tiles_grade_original_pixels_before_rotated_target_averaging() {
    use antitone::{
        crop::Crop,
        geometry::{FramePoint, PhotoOrientation, QuarterTurns, RollOrientation},
        resample::AreaAverage,
    };
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = antitone::renderer::GpuRenderer::new(&device, &queue)
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
        for corrected in [false, true] {
            let cal = if corrected {
                field.processing_calibration(calibration()).unwrap()
            } else {
                calibration()
            };
            let processing = input(&device, cal, adjustments());
            let global = Pipeline::new(
                cal,
                RollSettings::default(),
                PhotoSettings::default(),
                DMat3::IDENTITY,
            )
            .unwrap();
            let working: Vec<_> = (AREA.y..AREA.y + AREA.height)
                .flat_map(|y| {
                    (AREA.x..AREA.x + AREA.width)
                        .map(|x| {
                            let p = codes([x, y]);
                            let gain = if corrected {
                                field.gain([x as f64 + 0.5, y as f64 + 0.5]).unwrap()
                            } else {
                                DVec3::ONE
                            };
                            local_reference(
                                global
                                    .apply(
                                        DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.
                                            * gain,
                                    )
                                    .unwrap()
                                    .rgb,
                                [x, y],
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let rendered = CpuRenderer::new(target).unwrap().render(&working).unwrap();
                for angle in [-19_f64, 27.] {
                    let crop = Crop {
                        center: [131.5, 82.5],
                        size: [16, 12],
                        angle: angle.to_radians(),
                    }
                    .constrained(AREA)
                    .unwrap();
                    let composition = Composition::with_crop(
                        AREA,
                        RollOrientation {
                            turns: QuarterTurns::One,
                            horizontal: true,
                            ..Default::default()
                        },
                        PhotoOrientation::default(),
                        Some(crop),
                    )
                    .unwrap();
                    let frame = FrameRaster::new(
                        composition,
                        FramePoint([0., 0.]),
                        FramePoint(composition.size().map(f64::from)),
                        [5, 3],
                    )
                    .unwrap();
                    let bounds = frame.bounds().unwrap();
                    let average = AreaAverage::new_affine(&device, frame).await.unwrap();
                    let mut tile = GpuImage::new_shared(
                        &renderer,
                        &device,
                        6,
                        4,
                        &[[0; 4]; 24],
                        DMat3::IDENTITY,
                        target,
                    )
                    .await
                    .unwrap();
                    tile.set_flat_field(
                        corrected.then(|| gpu_field.clone()),
                        SourceGeometry::new([bounds.x, bounds.y, 6, 4], [6, 4]).unwrap(),
                    )
                    .unwrap();
                    for y in (0..bounds.height).step_by(4) {
                        for x in (0..bounds.width).step_by(6) {
                            let pixels: Vec<_> = (0..4)
                                .flat_map(|dy| {
                                    (0..6).map(move |dx| {
                                        if x + dx < bounds.width && y + dy < bounds.height {
                                            codes([bounds.x + x + dx, bounds.y + y + dy])
                                        } else {
                                            [0; 4]
                                        }
                                    })
                                })
                                .collect();
                            tile.write_source(&queue, &pixels).unwrap();
                            tile.set_geometry(
                                SourceGeometry::new([bounds.x + x, bounds.y + y, 6, 4], [6, 4])
                                    .unwrap(),
                            )
                            .unwrap();
                            let mut encoder = device.create_command_encoder(&Default::default());
                            tile.encode_processing(&device, &mut encoder, &processing)
                                .unwrap();
                            average.accumulate(&device, &mut encoder, &tile.linear, [x, y]);
                            queue.submit([encoder.finish()]);
                        }
                    }
                    let output = GpuImage::new_rendered_shared(&renderer, &device, 5, 3, target)
                        .await
                        .unwrap();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    average.finish(&device, &mut encoder, &output.linear);
                    queue.submit([encoder.finish()]);
                    let actual = read_image(&device, &queue, &output).await;
                    for y in 0..3 {
                        for x in 0..5 {
                            let footprint = frame.footprint([x, y]).unwrap();
                            let expected = footprint.pixels(AREA).unwrap().fold(
                                DVec3::ZERO,
                                |sum, ([sx, sy], weight)| {
                                    sum + rendered
                                        [((sy - AREA.y) * AREA.width + sx - AREA.x) as usize]
                                        .as_dvec3()
                                        * weight
                                        / footprint.area()
                                },
                            );
                            let pixel = actual[(y * 5 + x) as usize];
                            for c in 0..3 {
                                assert!(
                                    (f64::from(pixel[c]) - expected[c]).abs() < 2e-5,
                                    "corrected={corrected} target={target:?} angle={angle} pixel={x},{y}: {pixel:?} vs {expected:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    });
}
