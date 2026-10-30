use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    dust_repair::{Repair, RepairId, RepairOrigin, SupportMask},
    film_emulation::{FilmEmulationSettings, FilmPreset},
    gpu::repairs::{Patch, Snapshot},
    processing::InputPipeline,
};

fn input(device: &wgpu::Device, scan: &Scan, film: FilmEmulationSettings) -> ProcessingInput {
    ProcessingInput::from(Arc::new(
        ProcessingInput::new(
            device,
            InputSettings::Digital,
            Default::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
            scan.area,
        )
        .unwrap(),
    ))
    .with_film_emulation(film)
    .with_frame_size([scan.area.width, scan.area.height])
    .with_film_seed(17)
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    input: &ProcessingInput,
) -> Vec<[f32; 4]> {
    let view = RenderedView::new(device, source, input, OutputTarget::Srgb, None).unwrap();
    view.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    read(device, queue, &view.graded.as_ref().unwrap().pixels)
}

#[test]
fn small_source_composed_processing_accepts_and_executes_finite_grain() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let fixture = crate::dng_fixture::Fixture::default().write();
        let scan = Scan::open(fixture.path(), None).unwrap();
        assert!(
            scan.area.height <= 64,
            "exercise the formerly false-rejected small-source domain"
        );
        let composition =
            Composition::new(scan.area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::new(
            composition,
            FramePoint([8., 8.]),
            FramePoint([24., 24.]),
            [16, 16],
        )
        .unwrap();
        let settings = FilmPreset::ThirtyFiveMm.settings();
        let input = input(&device, &scan, settings);
        let spec = ViewSourceSpec::new(
            17,
            frame,
            InputSettings::Digital,
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
            None,
        )
        .unwrap()
        .with_pipeline(input.pixel_pipeline().unwrap())
        .unwrap();
        let source = ViewSource::acquire(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            None,
            spec,
            || false,
        )
        .unwrap()
        .unwrap();
        let clean = render(
            &device,
            &queue,
            source.clone(),
            &input.clone().with_film_emulation(FilmEmulationSettings {
                amount: 0.,
                ..settings
            }),
        );
        let grain = render(&device, &queue, source, &input);
        assert!(grain.iter().flatten().all(|v| v.is_finite()));
        assert!(
            grain
                .iter()
                .zip(clean)
                .any(|(a, b)| (a[1] - b[1]).abs() > 0.001),
            "composed processing must apply grain, not merely admit it"
        );
    });
}

#[test]
fn acquired_source_edges_match_clamped_native_gaussian_and_rotated_constants_stay_constant() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        for ramp in [true, false] {
            let fixture = crate::dng_fixture::Fixture {
                side: 1024,
                ramp,
                ..Default::default()
            }
            .write();
            let scan = Scan::open(fixture.path(), None).unwrap();
            let composition = if ramp {
                Composition::new(scan.area, Default::default(), Default::default()).unwrap()
            } else {
                Composition::with_crop(
                    scan.area,
                    Default::default(),
                    Default::default(),
                    Some(crate::crop::Crop {
                        center: [f64::from(scan.area.x) + 12., f64::from(scan.area.y) + 12.],
                        size: [18, 18],
                        angle: 0.21,
                    }),
                )
                .unwrap()
            };
            let mut frame = FrameRaster::full(composition).unwrap();
            if ramp {
                frame = FrameRaster::new(
                    composition,
                    FramePoint([0., 0.]),
                    FramePoint([16., 16.]),
                    [16, 16],
                )
                .unwrap();
            } else {
                frame.output = [27, 27];
            }
            let film = FilmEmulationSettings {
                enabled: true,
                size: 0.060,
                image_sigma: 0.060,
                amount: 0.,
                colour: 0.5,
            };
            let input = input(&device, &scan, film).with_frame_size(composition.crop().size);
            let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
            let spec = ViewSourceSpec::new(17, frame, InputSettings::Digital, matrix, None)
                .unwrap()
                .with_pipeline(input.pixel_pipeline().unwrap())
                .unwrap();
            let source = ViewSource::acquire(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                spec,
                || false,
            )
            .unwrap()
            .unwrap();
            let pixels = render(&device, &queue, source, &input);
            let scalar = InputPipeline::new(InputSettings::Digital, matrix).unwrap();
            if !ramp {
                let expected = scalar
                    .apply(scan.pixel(scan.area.x + 12, scan.area.y + 12).unwrap())
                    .unwrap()
                    .rgb;
                for p in pixels {
                    for c in 0..3 {
                        assert!(
                            (f64::from(p[c]) - expected[c]).abs() < 0.00002,
                            "rotated/partial source-edge footprint c={c}"
                        );
                    }
                }
                continue;
            }
            let sigma = film
                .parameters(frame.composition.crop().size)
                .unwrap()
                .unwrap()
                .image_mtf_sigma;
            let radius = (4. * sigma).ceil() as i32;
            let mass: f64 = (-radius..=radius)
                .map(|i| (-0.5 * f64::from(i * i) / (sigma * sigma)).exp())
                .sum();
            let mut distinguishes_truncation = false;
            for y in 0..16 {
                for x in 0..16 {
                    let mut expected = DVec3::ZERO;
                    let mut truncated = DVec3::ZERO;
                    let mut truncated_mass = 0.;
                    for dy in -radius..=radius {
                        for dx in -radius..=radius {
                            let weight =
                                (-0.5 * f64::from(dx * dx + dy * dy) / (sigma * sigma)).exp();
                            let sx = (scan.area.x as i32 + x + dx).clamp(
                                scan.area.x as i32,
                                (scan.area.x + scan.area.width - 1) as i32,
                            ) as u32;
                            let sy = (scan.area.y as i32 + y + dy).clamp(
                                scan.area.y as i32,
                                (scan.area.y + scan.area.height - 1) as i32,
                            ) as u32;
                            let value = scalar.apply(scan.pixel(sx, sy).unwrap()).unwrap().rgb;
                            expected += weight * value / (mass * mass);
                            if x + dx >= 0 && y + dy >= 0 {
                                truncated += weight * value;
                                truncated_mass += weight;
                            }
                        }
                    }
                    distinguishes_truncation |=
                        (expected - truncated / truncated_mass).abs().max_element() > 0.00001;
                    let actual = pixels[(y * 16 + x) as usize];
                    for c in 0..3 {
                        assert!(
                            (f64::from(actual[c]) - expected[c]).abs() < 0.000005,
                            "native source-edge Gaussian x={x} y={y} c={c}"
                        );
                    }
                }
            }
            assert!(
                distinguishes_truncation,
                "fixture must distinguish source clamping from skipped/renormalized neighbours"
            );
        }
    });
}

#[test]
fn padded_repair_reconstruction_preserves_layout_at_crop_and_source_edges() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let fixture = crate::dng_fixture::Fixture {
            side: 1024,
            ramp: true,
            ..Default::default()
        }
        .write();
        let scan = Scan::open(fixture.path(), None).unwrap();
        for at_source_edge in [false, true] {
            let point = [
                // The current healer needs two real exterior pixels around its support.
                scan.area.x + if at_source_edge { 2 } else { 40 },
                scan.area.y + 40,
            ];
            // An explicit neutral repair payload isolates raster reconstruction from healer fitting.
            let repair = Repair {
                id: RepairId(1),
                enabled: true,
                origin: RepairOrigin::Manual,
                offset: [32, 0],
                support: SupportMask::new(point, [3, 3], vec![true; 9]).unwrap(),
            };
            let evaluated = Arc::new(
                crate::dust_repair::evaluate(&repair, scan.area, |_| Ok([1.5, -0.2, 0.3])).unwrap(),
            );
            let patches = Snapshot(vec![Patch::new(&device, evaluated.clone()).unwrap()]);
            let composition = Composition::with_crop(
                scan.area,
                Default::default(),
                Default::default(),
                Some(crate::crop::Crop {
                    center: [
                        f64::from(point[0]) + if at_source_edge { 254. } else { 259. },
                        f64::from(point[1]) + 249.,
                    ],
                    size: [512, 512],
                    angle: 0.,
                }),
            )
            .unwrap();
            let frame = FrameRaster::new(
                composition,
                FramePoint([0., 0.]),
                FramePoint([16., 16.]),
                [16, 16],
            )
            .unwrap();
            if !at_source_edge {
                assert!(evaluated.support().selected_points().all(|p| {
                    frame
                        .composition
                        .source_to_frame(crate::geometry::SourcePoint(
                            p.map(|v| f64::from(v) + 0.5),
                        ))
                        .0[0]
                        < 0.
                }));
            }
            let film = FilmEmulationSettings {
                enabled: true,
                size: 0.100,
                image_sigma: 0.100,
                amount: 0.,
                colour: 0.5,
            };
            let mut input = input(&device, &scan, film).with_frame_size(composition.crop().size);
            input.repairs = patches.clone();
            let spec = ViewSourceSpec::new(
                17,
                frame,
                InputSettings::Digital,
                scan.positive_matrix(&InputSettings::Digital).unwrap(),
                None,
            )
            .unwrap()
            .with_pipeline(input.pixel_pipeline().unwrap())
            .unwrap();
            let acquire = |spec| {
                ViewSource::acquire(
                    &device,
                    &queue,
                    &SubmissionGate::default(),
                    renderer.clone(),
                    kernels.clone(),
                    &scan,
                    None,
                    spec,
                    || false,
                )
                .unwrap()
                .unwrap()
            };
            let base = acquire(spec.clone());
            let rebuilt = ViewSource::reconstruct_repairs(
                &device,
                &queue,
                &SubmissionGate::default(),
                &scan,
                None,
                base.clone(),
                patches.clone(),
                None,
                || false,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                rebuilt.pixels.size(),
                base.pixels.size(),
                "repair output keeps the padded input layout"
            );
            let direct = acquire(spec.with_repairs(patches));
            let actual = render(&device, &queue, rebuilt, &input);
            let expected = render(&device, &queue, direct, &input);
            for (a, b) in actual.iter().zip(expected) {
                for c in 0..4 {
                    assert!(
                        (a[c] - b[c]).abs() < 0.000003,
                        "reconstructed padded raster channel {c}"
                    );
                }
            }
            input.repairs = Snapshot::default();
            let clean = render(&device, &queue, base, &input);
            assert!(
                actual
                    .iter()
                    .zip(clean)
                    .any(|(a, b)| (a[0] - b[0]).abs() > 0.01),
                "repair support at a source edge or wholly outside composition must influence the filtered output"
            );
        }
    });
}
