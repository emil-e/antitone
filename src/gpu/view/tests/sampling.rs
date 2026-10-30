use super::*;

use crate::{
    halation::HalationParameters,
    image_mtf::ImageMtfParameters,
    pixel_pipeline::{EffectDefinition, PixelPipeline, PixelStage},
    processing_support::Support,
};

// Recreate the unrefined view composition with the same native effect operators. Only
// acquisition hints are omitted; this is a control, not a second photographic model.
#[derive(Debug, PartialEq)]
struct Unrefined(PixelStage);

impl EffectDefinition for Unrefined {
    fn support(&self, pitch: [f64; 2]) -> Support {
        self.0.support(pitch)
    }
    fn compose(
        &self,
        input: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        self.0.compose(input)
    }
}

fn input(device: &wgpu::Device, frame: FrameRaster, pipeline: PixelPipeline) -> ProcessingInput {
    ProcessingInput::new(
        device,
        InputSettings::Digital,
        RollSettings::default(),
        AdjustmentLayerSnapshot::new(vec![]).unwrap(),
        DMat3::IDENTITY,
        frame.composition.area(),
    )
    .unwrap()
    .with_pixel_pipeline(pipeline)
}

fn ready_source(
    device: &wgpu::Device,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    frame: FrameRaster,
    pipeline: PixelPipeline,
    frequency: f64,
) -> Arc<ViewSource> {
    let spec = ViewSourceSpec::new(17, frame, InputSettings::Digital, DMat3::IDENTITY, None)
        .unwrap()
        .with_pipeline(pipeline)
        .unwrap();
    let upstream = spec.input_frame().unwrap();
    let width = frame.max.0[0] - frame.min.0[0];
    let pitch = spec.plan.pitch()[0];
    let phase = std::f64::consts::PI * frequency * pitch / width;
    let aperture = if phase == 0. { 1. } else { phase.sin() / phase };
    let reference = frame.source_point([0.; 2]).0;
    let x_axis = crate::gpu::evaluation::Grid::from_frame(frame).axes[0];
    let length2 = x_axis.iter().map(|v| v * v).sum::<f64>();
    let size = upstream.output;
    let values: Vec<_> = (0..size[1])
        .flat_map(|y| (0..size[0]).map(move |x| [x, y]))
        .map(|p| {
            let point = upstream.source_point(p.map(|v| f64::from(v) + 0.5)).0;
            let local = (0..2)
                .map(|i| (point[i] - reference[i]) * x_axis[i])
                .sum::<f64>()
                / length2;
            let value = 0.5
                + 0.2
                    * aperture
                    * (std::f64::consts::TAU * frequency * local / f64::from(frame.output[0]))
                        .cos();
            [value as f32, value as f32, value as f32, 8.]
        })
        .collect();
    Arc::new(ViewSource {
        spec,
        renderer: renderer.clone(),
        kernels: kernels.clone(),
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("independently integrated sinusoidal neutral cells"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }),
    })
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    pipeline: PixelPipeline,
) -> Vec<[f32; 4]> {
    let input = input(device, source.spec.frame(), pipeline);
    let view = RenderedView::new(device, source, &input, OutputTarget::Srgb, None).unwrap();
    view.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    read(device, queue, &view.graded.as_ref().unwrap().pixels)
}

fn modulation(pixels: &[[f32; 4]], size: [u32; 2], frequency: f64) -> [f64; 3] {
    std::array::from_fn(|c| {
        let sum = pixels
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let x = i as u32 % size[0];
                (f64::from(p[c]) - 0.5)
                    * (std::f64::consts::TAU * frequency * (f64::from(x) + 0.5)
                        / f64::from(size[0]))
                    .cos()
            })
            .sum::<f64>();
        2. * sum / f64::from(size[0] * size[1])
    })
}

// Independent original-cell, finite-tap frequency response. This expectation never calls
// the GPU kernels, the view grid selector or the reconstruction implementation.
fn native_gaussian(sigma: f64, angular_frequency: f64) -> f64 {
    let radius = (4. * sigma).ceil() as i32;
    let weights: Vec<_> = (-radius..=radius)
        .map(|i| {
            let weight = (-0.5 * (f64::from(i) / sigma).powi(2)).exp();
            (weight, weight * (angular_frequency * f64::from(i)).cos())
        })
        .collect();
    weights.iter().map(|(_, v)| v).sum::<f64>() / weights.iter().map(|(w, _)| w).sum::<f64>()
}

#[test]
fn padded_f32_grids_near_zero_origins_preserve_valid_composed_mappings() {
    let (device, queue, renderer, kernels) = pollster::block_on(context());
    let composition = Composition::new(
        [0, 0, 240, 240].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let pipeline = PixelPipeline {
        stages: vec![
            PixelStage::new(ImageMtfParameters { sigma: 1.44 }),
            PixelStage::new(HalationParameters {
                sigma: 3.6,
                strength: 0.54,
            }),
        ],
    };
    for start in [0., 1e-5, 2.4] {
        let frame = FrameRaster::new(
            composition,
            FramePoint([start; 2]),
            FramePoint([240.; 2]),
            [50; 2],
        )
        .unwrap();
        let source = ready_source(&device, &renderer, &kernels, frame, pipeline.clone(), 3.);
        assert!(source.spec.plan.output_offset().unwrap()[0] > 0);
        let domain = crate::gpu::evaluation::Domain {
            bounds: [
                0,
                0,
                source.spec.input_frame().unwrap().output[0],
                source.spec.input_frame().unwrap().output[1],
            ]
            .into(),
            grid: crate::gpu::evaluation::Grid::from_raster(crate::gpu::raster::Raster::planned(
                &source.spec.plan,
            )),
            meaning: crate::gpu::evaluation::Meaning::Graded,
        };
        let output = crate::gpu::evaluation::Domain {
            bounds: [0, 0, 50, 50].into(),
            grid: crate::gpu::evaluation::Grid::from_raster(crate::gpu::raster::Raster::frame(
                frame,
            )),
            meaning: crate::gpu::evaluation::Meaning::Graded,
        };
        let offset = source.spec.plan.output_offset().unwrap().map(f64::from);
        let scale = std::array::from_fn(|i| f64::from(source.spec.plan.output().output[i]) / 50.);
        assert!(
            crate::gpu::evaluation::reconstruct_cells(
                crate::gpu::evaluation::borrowed_input(domain),
                output,
                [offset[0] + 0.25, offset[1]],
                scale,
            )
            .is_err(),
            "roundoff admission must still reject a genuinely displaced grid"
        );
        let pixels = render(&device, &queue, source, pipeline.clone());
        assert_eq!(pixels.len(), 2500);
        assert!(
            pixels
                .iter()
                .all(|p| p[3] == 8. && p[..3].iter().all(|v| v.is_finite()))
        );
    }
}

#[test]
fn refined_prepared_views_reuse_initialized_grain_and_preserve_pixels_without_retention() {
    let (device, queue, renderer, kernels) = pollster::block_on(context());
    let composition = Composition::new(
        [0, 0, 6000, 6000].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([1000.; 2]),
        FramePoint([1384.; 2]),
        [64; 2],
    )
    .unwrap();
    let film = crate::film_emulation::FilmPreset::ThirtyFiveMm.settings();
    let pipeline = |amount, colour, strength| {
        PixelPipeline::emulation(
            FilmEmulationSettings {
                amount,
                colour,
                ..film
            },
            crate::halation::HalationSettings {
                enabled: true,
                strength,
                ..Default::default()
            },
            composition.crop().size,
            17,
        )
        .unwrap()
    };
    let mut expected = None;
    let mut expected_scratch = None;
    for budget in [256u64 << 20, 0, 1] {
        let renderer = if budget == 256 << 20 {
            renderer.clone()
        } else {
            pollster::block_on(crate::renderer::GpuRenderer::new_with_cache_policy(
                &device,
                &queue,
                Default::default(),
                crate::gpu::evaluation::CachePolicy {
                    generated_bytes: budget,
                },
            ))
            .unwrap()
        };
        let p = pipeline(film.amount, film.colour, 0.2);
        let source = ready_source(&device, &renderer, &kernels, frame, p.clone(), 5.);
        assert!(source.spec.plan.output().output[0] > frame.output[0]);
        let view = RenderedView::new(
            &device,
            source.clone(),
            &input(&device, frame, p),
            OutputTarget::Srgb,
            None,
        )
        .unwrap();
        if budget > 1 {
            let field = view.graded.as_ref().unwrap().stages.field().unwrap();
            assert!(!field.ready());
            assert_eq!(
                field.pixels.size(),
                u64::from(frame.output[0] * frame.output[1]) * 16
            );
            let mut abandoned = device.create_command_encoder(&Default::default());
            kernels.grade(&device, &mut abandoned, &view.graded.as_ref().unwrap().data);
            drop(abandoned);
            assert!(
                !field.ready(),
                "recording alone must not initialize a generated value"
            );
        }
        view.ensure_submitted(
            &device,
            &queue,
            &Default::default(),
            &egui::Context::default(),
        );
        let initial = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
        let p = pipeline(0.09, 0.8, 0.8);
        let edited = RenderedView::new(
            &device,
            source,
            &input(&device, frame, p),
            OutputTarget::Srgb,
            Some(&view),
        )
        .unwrap();
        let scratch = edited.graded.as_ref().unwrap().stages.scratch_bytes();
        if let Some(expected) = expected_scratch {
            assert_eq!(
                scratch, expected,
                "retention does not change required work or its grid"
            );
        } else {
            expected_scratch = Some(scratch);
        }
        let a = view.graded.as_ref().unwrap().stages.field();
        let b = edited.graded.as_ref().unwrap().stages.field();
        if budget > 1 {
            assert!(Arc::ptr_eq(a.unwrap(), b.unwrap()));
            assert!(
                b.unwrap().ready(),
                "Amount, Colour and fixed-support halation reuse the initialized field"
            );
        } else {
            assert!(a.is_none() && b.is_none());
        }
        edited.ensure_submitted(
            &device,
            &queue,
            &Default::default(),
            &egui::Context::default(),
        );
        let pixels = read(&device, &queue, &edited.graded.as_ref().unwrap().pixels);
        assert!(
            pixels
                .iter()
                .zip(initial)
                .any(|(a, b)| (a[0] - b[0]).abs() > 0.001)
        );
        assert!(
            pixels
                .iter()
                .all(|p| p[3] == 8. && p[..3].iter().all(|v| v.is_finite()))
        );
        if let Some(expected) = &expected {
            assert_eq!(&pixels, expected);
        } else {
            expected = Some(pixels);
        }
        if budget <= 1 {
            assert_eq!(renderer.evaluation().retained_bytes(), 0);
        }
    }
}

#[test]
fn refined_original_acquisition_preserves_region_seams_real_neighbours_and_native_edges() {
    let (device, queue, renderer, kernels) = pollster::block_on(context());
    let fixture = crate::dng_fixture::Fixture {
        side: 1024,
        ramp: true,
        marker: Some((503, 503, 60000)),
        ..Default::default()
    }
    .write();
    let scan = Scan::open(fixture.path(), None).unwrap();
    let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
    for rotated in [false, true] {
        let composition = Composition::with_crop(
            scan.area,
            crate::geometry::RollOrientation {
                turns: if rotated {
                    crate::geometry::QuarterTurns::One
                } else {
                    Default::default()
                },
                horizontal: rotated,
                vertical: false,
            },
            Default::default(),
            rotated.then_some(crate::crop::Crop {
                center: [f64::from(scan.area.x) + 512., f64::from(scan.area.y) + 512.],
                size: [768, 640],
                angle: 0.19,
            }),
        )
        .unwrap();
        let mut frame = FrameRaster::full(composition).unwrap();
        frame.output = if rotated { [96, 80] } else { [64, 64] };
        if !rotated {
            // The authored DNG has an active-area border. Use an integral native/output
            // ratio established from that actual area, retaining its left/top source edges.
            frame.max.0 = composition.size().map(|n| f64::from(n / 64 * 64));
        }
        let pipeline = PixelPipeline::emulation(
            FilmEmulationSettings {
                enabled: true,
                amount: 0.,
                ..Default::default()
            },
            crate::halation::HalationSettings {
                enabled: true,
                ..Default::default()
            },
            composition.crop().size,
            17,
        )
        .unwrap();
        let make_input = |pipeline| {
            ProcessingInput::new(
                &device,
                InputSettings::Digital,
                Default::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                matrix,
                scan.area,
            )
            .unwrap()
            .with_pixel_pipeline(pipeline)
        };
        let acquire = |frame, pipeline: PixelPipeline| {
            ViewSource::acquire(
                &device,
                &queue,
                &Default::default(),
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                ViewSourceSpec::new(17, frame, InputSettings::Digital, matrix, None)
                    .unwrap()
                    .with_pipeline(pipeline)
                    .unwrap(),
                || false,
            )
            .unwrap()
            .unwrap()
        };
        let render = |source, pipeline| {
            let view = RenderedView::new(
                &device,
                source,
                &make_input(pipeline),
                OutputTarget::Srgb,
                None,
            )
            .unwrap();
            view.ensure_submitted(
                &device,
                &queue,
                &Default::default(),
                &egui::Context::default(),
            );
            read(&device, &queue, &view.graded.as_ref().unwrap().pixels)
        };
        let source = acquire(frame, pipeline.clone());
        assert!(source.spec.plan.output().output[0] > frame.output[0]);
        let whole = render(source, pipeline.clone());
        for right in [false, true] {
            let middle = (frame.min.0[0] + frame.max.0[0]) * 0.5;
            let mut tile = frame;
            tile.output[0] /= 2;
            if right {
                tile.min.0[0] = middle;
            } else {
                tile.max.0[0] = middle;
            }
            let part = render(acquire(tile, pipeline.clone()), pipeline.clone());
            for y in 0..tile.output[1] {
                for x in 0..tile.output[0] {
                    let at =
                        (y * frame.output[0] + x + if right { tile.output[0] } else { 0 }) as usize;
                    let actual = part[(y * tile.output[0] + x) as usize];
                    for c in 0..3 {
                        assert!(
                            (actual[c] - whole[at][c]).abs() < 2e-5,
                            "real-neighbour seam right={right} rotated={rotated} ({x},{y}) c={c}: part {} whole {} delta {}",
                            actual[c],
                            whole[at][c],
                            (actual[c] - whole[at][c]).abs()
                        );
                    }
                    assert_eq!(actual[3], whole[at][3]);
                }
            }
        }
        if !rotated {
            let mut native_frame = frame;
            native_frame.output = frame.max.0.map(|v| v as u32);
            assert!((0..2).all(|i| native_frame.output[i].is_multiple_of(frame.output[i])));
            let native = render(acquire(native_frame, pipeline.clone()), pipeline.clone());
            let old = PixelPipeline {
                stages: pipeline
                    .stages
                    .into_iter()
                    .map(|s| PixelStage::new(Unrefined(s)))
                    .collect(),
            };
            let coarse = render(acquire(frame, old.clone()), old);
            // Integer native/output cells provide an independent f64 box oracle here.
            let factor = native_frame.output[0] / frame.output[0];
            let error = |pixels: &[[f32; 4]]| {
                let mut total = 0.;
                for y in 0..frame.output[1] {
                    for x in 0..frame.output[0] {
                        for c in 0..3 {
                            let mut expected = 0.;
                            for dy in 0..factor {
                                for dx in 0..factor {
                                    expected += f64::from(
                                        native[((y * factor + dy) * native_frame.output[0]
                                            + x * factor
                                            + dx)
                                            as usize][c],
                                    ) / f64::from(factor * factor);
                                }
                            }
                            total += (f64::from(pixels[(y * frame.output[0] + x) as usize][c])
                                - expected)
                                .powi(2);
                        }
                    }
                }
                (total / f64::from(frame.output[0] * frame.output[1] * 3)).sqrt()
            };
            let new_error = error(&whole);
            let old_error = error(&coarse);
            assert!(
                old_error > 1e-7,
                "bright native-cell boundary marker must distinguish omitted filtering: old={old_error} new={new_error}"
            );
            assert!(
                new_error < old_error * 0.2,
                "original acquisition new={new_error} old={old_error}"
            );
        }
    }
}

#[test]
fn selected_view_resolves_subpixel_mtf_and_independent_halation_toward_native_response() {
    let (device, queue, renderer, kernels) = pollster::block_on(context());
    for side in [4096, 8192] {
        for output in [257, 512] {
            let composition = Composition::new(
                [37, 29, side, side].into(),
                Default::default(),
                Default::default(),
            )
            .unwrap();
            let frame = FrameRaster::new(
                composition,
                FramePoint([128., 100.]),
                FramePoint([f64::from(side) - 128., 116.]),
                [output, 5],
            )
            .unwrap();
            let mtf = ImageMtfParameters {
                sigma: f64::from(side) * 0.0004,
            };
            let halo = HalationParameters {
                sigma: f64::from(side) * 0.00109,
                strength: 0.54,
            };
            for frequency in [32., (0.45 * f64::from(output)).floor()] {
                for (name, stages) in [
                    ("mtf", vec![PixelStage::new(mtf)]),
                    ("halo", vec![PixelStage::new(halo)]),
                    ("both", vec![PixelStage::new(mtf), PixelStage::new(halo)]),
                ] {
                    let pipeline = PixelPipeline { stages };
                    let old = PixelPipeline {
                        stages: pipeline
                            .stages
                            .iter()
                            .cloned()
                            .map(|s| PixelStage::new(Unrefined(s)))
                            .collect(),
                    };
                    let source = ready_source(
                        &device,
                        &renderer,
                        &kernels,
                        frame,
                        pipeline.clone(),
                        frequency,
                    );
                    assert_eq!(source.spec.frame(), frame);
                    assert!(source.spec.plan.output().output[0] > output);
                    let clean = modulation(
                        &render(&device, &queue, source.clone(), PixelPipeline::default()),
                        frame.output,
                        frequency,
                    );
                    let actual = modulation(
                        &render(&device, &queue, source, pipeline),
                        frame.output,
                        frequency,
                    );
                    let old_source =
                        ready_source(&device, &renderer, &kernels, frame, old.clone(), frequency);
                    let old_clean = modulation(
                        &render(
                            &device,
                            &queue,
                            old_source.clone(),
                            PixelPipeline::default(),
                        ),
                        frame.output,
                        frequency,
                    );
                    let old_actual = modulation(
                        &render(&device, &queue, old_source, old),
                        frame.output,
                        frequency,
                    );
                    let phase = std::f64::consts::PI * frequency / f64::from(output);
                    let aperture = 0.2 * phase.sin() / phase;
                    // Integral subdivisions must preserve the destination aperture. Isolate
                    // that control from effect response rather than hiding an extra pixel blur.
                    assert!(
                        clean.iter().all(|v| (*v - aperture).abs() < 2e-5),
                        "clean {side} {output} {frequency}: {clean:?} / {aperture}"
                    );
                    assert!(old_clean.iter().all(|v| (*v - aperture).abs() < 2e-5));
                    let angular = std::f64::consts::TAU * frequency / (f64::from(side) - 256.);
                    let expected: [f64; 3] = std::array::from_fn(|c| {
                        let m = if name != "halo" {
                            native_gaussian(mtf.sigma, angular)
                        } else {
                            1.
                        };
                        let h = if name != "mtf" {
                            1. + halo.fractions()[c] * (native_gaussian(halo.sigma, angular) - 1.)
                        } else {
                            1.
                        };
                        m * h
                    });
                    for c in 0..3 {
                        let response = actual[c] / clean[c];
                        let old_response = old_actual[c] / old_clean[c];
                        let error = (response - expected[c]).abs();
                        assert!(
                            error < 0.012,
                            "{name} side={side} output={output} f={frequency} c={c}: new {response}, native {}, old {old_response}",
                            expected[c]
                        );
                        if name != "halo" && frequency > 100. {
                            assert!(
                                error < (old_response - expected[c]).abs() * 0.2,
                                "refinement must materially improve MTF"
                            );
                        }
                    }
                    println!(
                        "sampling {name} side={side} output={output} f={frequency}: native={expected:?} new={:?} old={:?}",
                        std::array::from_fn::<_, 3, _>(|c| actual[c] / clean[c]),
                        std::array::from_fn::<_, 3, _>(|c| old_actual[c] / old_clean[c])
                    );
                }
            }
        }
    }
}
