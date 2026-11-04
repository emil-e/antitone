//! Stage factoring and asymmetric source coverage on production working views.
use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
use crate::gpu::submission::SubmissionGate;

#[test]
fn resident_acquisition_inverts_before_fractional_coverage_with_rounded_proxy_dimensions() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let area: SourceRegion = [8, 12, 6000, 4000].into();
        let size = [31, 21];
        let codes: Vec<[u16; 4]> = (0..651)
            .map(|i| {
                let code = if i % 31 < 15 { 52428 } else { 13107 };
                [code, code, code, 0]
            })
            .collect();
        let source = ProcessingSource::new(
            &device,
            &codes,
            DMat3::IDENTITY,
            Some(SourceGeometry::new([8, 12, 6000, 4000], size).unwrap()),
        )
        .unwrap();

        let source = Arc::new(source);
        let input = InputSettings::negative(Calibration {
            dmin: DVec3::splat(0.8),
            dmax: DVec3::splat(0.2),
            input_floor: 0.01,
        });
        for (region, expected) in [
            (area, 16. / 31.),
            ([1508, 1012, 3000, 2000].into(), 8.25 / 15.5),
        ] {
            let composition = Composition::with_crop(
                area,
                Default::default(),
                Default::default(),
                Some(crate::crop::Crop::from_region(region)),
            )
            .unwrap();
            let mut frame = FrameRaster::full(composition).unwrap();
            frame.output = [1, 1];
            let spec = SourceRequest::new(frame, input, DMat3::IDENTITY, None).unwrap();
            let image = ViewSource::from_processing(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                source.clone(),
                &spec.processing_fixture(),
                spec,
            )
            .unwrap();
            let actual = read(&device, &queue, &image.pixels);
            // The two source values are the fixed 0/1 calibration anchors. Only area weights vary.
            for value in &actual[0][..3] {
                assert!(
                    (f64::from(*value) - expected).abs() < 2e-6,
                    "{actual:?} expected={expected}"
                );
            }
            assert_eq!(actual[0][3], 8.);
        }
    });
}

#[test]
fn digital_source_marks_neutral_coverage() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let matrix = DMat3::from_cols(
            DVec3::new(0.6062, 0.0161, -0.0031),
            DVec3::new(0.3802, 1.1927, -0.1490),
            DVec3::new(0.0136, -0.2088, 1.1521),
        );
        let area = SourceRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        };
        let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::full(composition).unwrap();
        let pixels = vec![[4000., 5000., 6000., 0.]; 32];
        let source = ProcessingSource::new_continuous(
            &device,
            &pixels,
            matrix,
            Some(SourceGeometry::new([0, 0, 8, 4], [8, 4]).unwrap()),
        )
        .unwrap();

        let spec = SourceRequest::new(frame, InputSettings::Digital, matrix, None).unwrap();
        let source = ViewSource::from_processing(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            Arc::new(source),
            &spec.processing_fixture(),
            spec,
        )
        .unwrap();
        let neutral = read(&device, &queue, &source.pixels);
        assert!(neutral.iter().all(|pixel| (pixel[3] as u32) & 8 != 0));
    });
}

#[test]
fn working_positive_applies_roll_and_layer_grading_once_for_every_input_stage() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let area = SourceRegion {
            x: 12,
            y: 7,
            width: 3,
            height: 1,
        };
        let frame = FrameRaster::full(
            Composition::new(area, Default::default(), Default::default()).unwrap(),
        )
        .unwrap();
        let camera = [
            DVec3::splat(0.02),
            DVec3::new(0.08, 0.12, 0.16),
            DVec3::splat(0.6),
        ];
        for (settings, normalization) in [
            (InputSettings::Digital, 1.),
            (InputSettings::Digital, 2.25_f64.exp2()),
            (negative(0.65), 1.),
        ] {
            let matrix = DMat3::IDENTITY * normalization;
            let neutral = crate::processing::InputPipeline::new(settings, matrix).unwrap();
            let pixels: Vec<[f32; 4]> = camera
                .iter()
                .map(|&sample| {
                    let rgb = neutral.apply(sample).unwrap().rgb.as_vec3();
                    [rgb.x, rgb.y, rgb.z, 8.]
                })
                .collect();
            let source = Arc::new(ViewSource::from_neutral_samples(
                SourceRequest::new(frame, settings, matrix, None).unwrap(),
                renderer.clone(),
                kernels.clone(),
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("scalar ungraded working positives"),
                    contents: bytemuck::cast_slice(&pixels),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                }),
            ));
            for (roll_ev, photo_ev) in [(1.5, 0.), (0., -1.), (-0.5, 1.25)] {
                for colour in [false, true] {
                    let roll = RollSettings {
                        exposure: roll_ev,
                        wb: if colour {
                            Balance {
                                warmth: 0.08,
                                tint: -0.04,
                            }
                        } else {
                            Balance::default()
                        },
                    };
                    let photo = AdjustmentLayerSettings {
                        exposure: photo_ev,
                        ..if colour {
                            AdjustmentLayerSettings {
                                wb: Balance {
                                    warmth: -0.12,
                                    tint: 0.06,
                                },
                                contrast: 1.1,
                                saturation: 0.8,
                                saturation_density: 0.4,
                                ..Default::default()
                            }
                        } else {
                            AdjustmentLayerSettings::default()
                        }
                    };
                    let snapshot = AdjustmentLayerSnapshot::new(
                        std::iter::once(crate::adjustment_layers::AdjustmentLayer::global(photo))
                            .chain(Vec::new())
                            .collect(),
                    )
                    .unwrap();
                    let rendered = render_with_basis(
                        &device,
                        &queue,
                        source.clone(),
                        snapshot,
                        OutputTarget::Srgb,
                        settings,
                        roll,
                    );
                    let actual = read(&device, &queue, &rendered.graded.as_ref().unwrap().pixels);
                    let cpu = crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                        crate::adjustment_layers::AdjustmentLayer::global(photo),
                    ])
                    .and_then(|layers| {
                        crate::adjustment_layers::AdjustmentLayerPipeline::new(
                            settings, roll, layers, matrix,
                        )
                    })
                    .unwrap();
                    for (&sample, pixel) in camera.iter().zip(actual) {
                        let expected = cpu.apply(sample, [0, 0]).unwrap().rgb;
                        assert_eq!(pixel[3], 8., "grading must preserve source coverage");
                        let actual = DVec3::new(pixel[0].into(), pixel[1].into(), pixel[2].into());
                        assert!(
                            (actual - expected).abs().max_element()
                                < 2e-5 * expected.abs().max_element().max(1.),
                            "{settings:?}, roll={roll:?}, photo={photo:?}: {actual:?} vs {expected:?}",
                        );
                    }
                }
            }
        }
    });
}

#[test]
fn straightening_preserves_coverage_with_rounded_proxy_dimensions() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let area = SourceRegion {
            x: 8,
            y: 12,
            width: 6000,
            height: 4000,
        };
        let size = [31, 21];
        let pixels = vec![[12000., 12000., 12000., 0.]; (size[0] * size[1]) as usize];
        let source = ProcessingSource::new_continuous(
            &device,
            &pixels,
            DMat3::IDENTITY,
            Some(SourceGeometry::new([8, 12, 6000, 4000], size).unwrap()),
        )
        .unwrap();

        let source = Arc::new(source);
        for degrees in [0.4_f64, 23., 45., 89.] {
            let crop = crate::crop::Crop::from_region(area)
                .straightened(area, degrees.to_radians())
                .unwrap();
            let composition =
                Composition::with_crop(area, Default::default(), Default::default(), Some(crop))
                    .unwrap();
            let mut frame = FrameRaster::full(composition).unwrap();
            frame.output = size;
            let spec = SourceRequest::inspection_fixture(frame, DMat3::IDENTITY, None).unwrap();
            let composed = ViewSource::from_processing(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                source.clone(),
                &spec.processing_fixture(),
                spec,
            )
            .unwrap();
            let values = read(&device, &queue, &composed.pixels);
            assert_eq!(values.len(), (size[0] * size[1]) as usize);
            for pixel in values {
                assert!(
                    (pixel[0] - 12000. / 65535.).abs() < 2e-5,
                    "angle={degrees}: {pixel:?}"
                );
                assert_eq!(pixel[3] as u32 & 8, 8);
            }
        }
    });
}

#[test]
fn crops_sample_both_source_halves_with_a_nonzero_active_origin() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let width = 8;
        let height = 4;
        let mut pixels = Vec::new();
        for _y in 0..height {
            for x in 0..width {
                pixels.push(if x < 4 {
                    [40000., 40000., 40000., 0.]
                } else {
                    [400., 400., 400., 0.]
                });
            }
        }
        let area = SourceRegion {
            x: 12,
            y: 7,
            width,
            height,
        };
        let source = ProcessingSource::new_continuous(
            &device,
            &pixels,
            DMat3::IDENTITY,
            Some(SourceGeometry::new([12, 7, width, height], [width, height]).unwrap()),
        )
        .unwrap();

        let source = Arc::new(source);
        for (offset, code) in [(0, 40000.), (4, 400.)] {
            let composition = Composition::with_crop(
                area,
                Default::default(),
                Default::default(),
                Some(crate::crop::Crop::from_region(SourceRegion {
                    x: area.x + offset,
                    y: area.y,
                    width: 4,
                    height: 4,
                })),
            )
            .unwrap();
            let frame = FrameRaster::full(composition).unwrap();
            let spec = SourceRequest::inspection_fixture(frame, DMat3::IDENTITY, None).unwrap();
            let composed = ViewSource::from_processing(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                source.clone(),
                &spec.processing_fixture(),
                spec,
            )
            .unwrap();
            let values = read(&device, &queue, &composed.pixels);
            for pixel in values {
                assert!(
                    (pixel[0] - code / 65535.).abs() < 1e-6,
                    "offset={offset}: {pixel:?}"
                );
                assert_eq!(pixel[3] as u32 & 8, 8);
            }
        }
    });
}
