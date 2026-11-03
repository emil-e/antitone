use super::*;
use crate::gpu::ProcessingParameters;
#[cfg(feature = "ocio-reference")]
use crate::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot, GradingPipeline,
    },
    crop::Crop,
    frame_raster::FrameRaster,
    geometry::{Composition, FramePoint},
    gpu::ProcessingInput,
    processing::{InputSettings, RollSettings},
};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[test]
fn regional_delivery_uses_the_authored_signed_grid_and_places_it_at_texture_origin() {
    struct Pixels(evaluation::Domain);
    impl evaluation::Source for Pixels {
        fn domain(&self) -> evaluation::Domain {
            self.0
        }
        fn read(&self, region: evaluation::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            assert!(self.0.contains_region(region));
            Ok(vec![
                [0., 0., 0., 8.];
                region.width as usize * region.height as usize
            ])
        }
    }
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let domain = evaluation::Domain {
        bounds: Some(evaluation::CellRegion::new([-2, 3], [2, 2])),
        grid: evaluation::Grid {
            coverage: 8,
            ..evaluation::Grid::axis_aligned([0.; 2], [1.; 2])
        },
        meaning: evaluation::Meaning::Display,
    };
    let root = evaluation::pointwise(
        evaluation::source(Arc::new(Pixels(domain))),
        evaluation::Meaning::Display,
        evaluation::Function {
            code: "fn coordinates(p: vec4f, xy: vec2f) -> vec4f { return vec4f(xy, 0.5, p.w); }"
                .into(),
            symbol: "coordinates",
            bindings: vec![],
        },
    );
    let output = pollster::block_on(crate::viewer::GpuImage::new_rendered_shared(
        &renderer,
        &device,
        2,
        2,
        OutputTarget::Srgb,
    ))
    .unwrap();
    assert!(
        execute(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
            root,
            false,
            &output.linear,
            || false
        )
        .unwrap()
        .is_some()
    );
    assert_eq!(
        crate::viewer::readback::image_pixels(&device, &queue, &output),
        vec![
            [-1.5, 3.5, 0.5, 1.],
            [-0.5, 3.5, 0.5, 1.],
            [-1.5, 4.5, 0.5, 1.],
            [-0.5, 4.5, 0.5, 1.],
        ]
    );
}

#[test]
fn regional_cancellation_observes_the_actual_submission_boundary_without_finishing_obsolete_work() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 768,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Scan::open(file.path(), None).unwrap();
    let generation = Arc::new(AtomicU64::new(7));
    let count = Arc::new(AtomicUsize::new(0));
    let gate = SubmissionGate::default();
    let (changed, observed) = (generation.clone(), count.clone());
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
        changed.store(8, Ordering::Relaxed);
    }));
    let demand = Demand {
        area: scan.area,
        output: [37, 29],
        frame: None,
    };
    let result = crate::refinement::refine(
        &renderer,
        &device,
        (&queue, &gate),
        (&scan, None),
        ProcessingParameters::source(),
        OutputTarget::Srgb,
        (&generation, 7),
        demand,
    )
    .unwrap();
    assert!(result.is_none());
    assert_eq!(
        count.load(Ordering::Relaxed),
        1,
        "no later source unit or final output may be submitted after cancellation"
    );
    assert!(
        crate::refinement::refine(
            &renderer,
            &device,
            (&queue, &gate),
            (&scan, None),
            ProcessingParameters::source(),
            OutputTarget::Srgb,
            (&generation, 7),
            demand
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        count.load(Ordering::Relaxed),
        1,
        "already cancelled demand does not acquire or submit"
    );
}

#[test]
fn reference_output_larger_than_a_storage_binding_keeps_its_full_grid_and_places_tail_tiles() {
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 1 << 20,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let renderer = pollster::block_on(GpuRenderer::new(&device, &queue)).unwrap();
    let file = crate::dng_fixture::Fixture {
        side: 64,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Scan::open(file.path(), None).unwrap();
    let composition =
        crate::geometry::Composition::new(scan.area, Default::default(), Default::default())
            .unwrap();
    let size = composition.size();
    let output = [515, 259];
    assert!(
        u64::from(output[0]) * u64::from(output[1]) * 16
            > device.limits().max_storage_buffer_binding_size,
        "control: a full-output storage binding must actually exceed this device's allowance"
    );
    let full = crate::frame_raster::FrameRaster::new(
        composition,
        crate::geometry::FramePoint([0.; 2]),
        crate::geometry::FramePoint(size.map(f64::from)),
        output,
    )
    .unwrap();
    let run = |frame: crate::frame_raster::FrameRaster| {
        crate::refinement::refine(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
            (&scan, None),
            ProcessingParameters::source(),
            OutputTarget::Srgb,
            (&AtomicU64::new(7), 7),
            Demand {
                area: frame.bounds().unwrap(),
                output: frame.output,
                frame: Some(frame),
            },
        )
        .unwrap()
        .unwrap()
    };
    let image = run(full);
    let pixels = crate::viewer::readback::image_pixels(&device, &queue, &image);
    assert_eq!(pixels.len(), output[0] as usize * output[1] as usize);
    let mut samples = Vec::new();
    for point in [[0, 0], [511, 127], [512, 128], [514, 258]] {
        let frame = crate::frame_raster::FrameRaster::new(
            composition,
            crate::geometry::FramePoint(std::array::from_fn(|i| {
                f64::from(size[i]) * f64::from(point[i]) / f64::from(output[i])
            })),
            crate::geometry::FramePoint(std::array::from_fn(|i| {
                f64::from(size[i]) * f64::from(point[i] + 1) / f64::from(output[i])
            })),
            [1, 1],
        )
        .unwrap();
        let expected = crate::viewer::readback::image_pixels(&device, &queue, &run(frame));
        let actual = pixels[(point[1] * output[0] + point[0]) as usize];
        for channel in 0..3 {
            assert!(
                (actual[channel] - expected[0][channel]).abs() < 2e-6,
                "output {point:?} channel {channel}: {actual:?} versus independent-position request {:?}",
                expected[0]
            );
        }
        samples.push(actual);
    }
    assert!(
        samples.windows(2).any(|pair| pair[0][..3] != pair[1][..3]),
        "control: contrasting source samples must expose shifted or missing tail writes"
    );
}

#[cfg(feature = "ocio-reference")]
#[test]
fn regional_original_grade_gaussian_and_display_precede_rotated_reconstruction_across_source_units()
{
    use glam::DVec3;

    const TOLERANCE: f64 = 2e-5;
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 768,
        bits: 16,
        ramp: true,
        marker: Some((512, 256, 60000)),
        ..Default::default()
    }
    .write();
    let scan = Scan::open(file.path(), None).unwrap();
    let a = scan.area;
    let composition = Composition::with_crop(
        a,
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [
                f64::from(a.x) + f64::from(a.width) / 2.,
                f64::from(a.y) + f64::from(a.height) / 2.,
            ],
            size: [620, 480],
            angle: 0.19,
        }),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([0.25, 0.75]),
        FramePoint([610.25, 470.75]),
        [147, 113],
    )
    .unwrap();
    let bounds = frame.bounds().unwrap();
    assert!(
        bounds.width > 512 && bounds.height > 128,
        "fixture must cross both native traversal dimensions"
    );
    let roll = RollSettings {
        exposure: 0.,
        ..Default::default()
    };
    let layers =
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 1.4,
            saturation: 1.1,
            ..Default::default()
        })])
        .unwrap();
    let source = crate::scan::ImageSource::new(&scan, InputSettings::Digital).unwrap();
    let gpu = Arc::new(
        ProcessingInput::new(
            &device.limits(),
            InputSettings::Digital,
            roll,
            layers.clone(),
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
            a,
        )
        .unwrap(),
    );
    let input = ProcessingInput::from(gpu).with_pixel_pipeline(
        crate::pixel_pipeline::PixelPipeline::emulation(
            crate::grain::GrainSettings {
                enabled: true,
                size: 0.1,
                image_sigma: 0.1,
                amount: 0.,
                colour: 0.5,
            },
            Default::default(),
            [620, 480],
            17,
        )
        .unwrap(),
    );
    let grading = GradingPipeline::new(roll, layers).unwrap();
    let original: Vec<_> = (a.y..a.y + a.height)
        .flat_map(|y| (a.x..a.x + a.width).map(move |x| [x, y]))
        .map(|point| grading.apply(source.sample(point).unwrap(), point).unwrap())
        .collect();
    // Independent whole-source separable Gaussian, sigma = 0.1% of the 480-cell accepted short side.
    let weights: Vec<_> = (-2..=2)
        .map(|i| (-0.5 * f64::from(i * i) / 0.48_f64.powi(2)).exp())
        .collect();
    let mass: f64 = weights.iter().sum();
    let mut blurred = original.clone();
    for axis in 0..2 {
        let previous = blurred.clone();
        for y in 0..a.height {
            for x in 0..a.width {
                let mut value = DVec3::ZERO;
                for (i, weight) in (-2..=2).zip(&weights) {
                    let p = if axis == 0 {
                        [(x as i32 + i).clamp(0, a.width as i32 - 1) as u32, y]
                    } else {
                        [x, (y as i32 + i).clamp(0, a.height as i32 - 1) as u32]
                    };
                    value += previous[(p[1] * a.width + p[0]) as usize] * (weight / mass);
                }
                blurred[(y * a.width + x) as usize] = value;
            }
        }
    }
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
        let rendered = cpu.render(&blurred).unwrap();
        let unfiltered = cpu.render(&original).unwrap();
        let mut order_difference = 0_f64;
        let mut blur_difference = 0_f64;
        for output in [[147, 113], [1, 1], [515, 131]] {
            let frame = FrameRaster { output, ..frame };
            if output == [1, 1] {
                let original = crate::gpu::evaluation::original_chain(
                    &device,
                    &renderer,
                    &scan,
                    None,
                    &input,
                    crate::rendering::OutputReference::DisplayReferred(target),
                    &input.pixel_pipeline(),
                )
                .unwrap();
                let domain = crate::gpu::evaluation::Domain {
                    bounds: Some([0, 0, 1, 1].into()),
                    grid: crate::gpu::evaluation::Grid::from_frame(frame),
                    meaning: original.domain().meaning,
                };
                let root = crate::gpu::evaluation::reconstruct(original, domain).unwrap();
                let evaluation = renderer
                    .evaluation()
                    .request(
                        root,
                        crate::gpu::evaluation::Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        crate::gpu::evaluation::Budget {
                            tile: [1, 1],
                            scratch_bytes: 32 << 20,
                        },
                    )
                    .unwrap();
                assert!(
                    renderer
                        .evaluation()
                        .record(
                            evaluation,
                            renderer.evaluation().materializations(),
                            Vec::new()
                        )
                        .is_err(),
                    "control: this aperture must require worker production rather than an ordinary recorded dispatch"
                );
            }
            let actual = crate::refinement::refine(
                &renderer,
                &device,
                (&queue, &SubmissionGate::default()),
                (&scan, None),
                input.clone(),
                target,
                (&AtomicU64::new(7), 7),
                Demand {
                    area: bounds,
                    output: frame.output,
                    frame: Some(frame),
                },
            )
            .unwrap()
            .unwrap();
            let pixels = crate::viewer::readback::image_pixels(&device, &queue, &actual);
            for y in 0..frame.output[1] {
                for x in 0..frame.output[0] {
                    let cells: Vec<_> = frame
                        .footprint([x, y])
                        .unwrap()
                        .pixels(a)
                        .unwrap()
                        .collect();
                    let expected =
                        crate::sampling::weighted_mean_iter(cells.iter().map(|(p, weight)| {
                            Ok((
                                rendered[((p[1] - a.y) * a.width + p[0] - a.x) as usize].as_dvec3(),
                                *weight,
                            ))
                        }))
                        .unwrap();
                    let without_mtf =
                        crate::sampling::weighted_mean_iter(cells.iter().map(|(p, weight)| {
                            Ok((
                                unfiltered[((p[1] - a.y) * a.width + p[0] - a.x) as usize]
                                    .as_dvec3(),
                                *weight,
                            ))
                        }))
                        .unwrap();
                    blur_difference =
                        blur_difference.max((expected - without_mtf).abs().max_element());
                    let working =
                        crate::sampling::weighted_mean_iter(cells.iter().map(|(p, weight)| {
                            Ok((
                                blurred[((p[1] - a.y) * a.width + p[0] - a.x) as usize],
                                *weight,
                            ))
                        }))
                        .unwrap();
                    let reversed = cpu.render(&[working]).unwrap()[0].as_dvec3();
                    order_difference =
                        order_difference.max((expected - reversed).abs().max_element());
                    let actual = glam::Vec3::from_array(
                        pixels[(y * frame.output[0] + x) as usize][..3]
                            .try_into()
                            .unwrap(),
                    )
                    .as_dvec3();
                    assert!(
                        (expected - actual).abs().max_element() < TOLERANCE,
                        "{target:?} output={output:?} ({x},{y}) expected={expected:?} actual={actual:?}"
                    );
                }
            }
        }
        assert!(
            blur_difference > 2. * TOLERANCE,
            "fixture must distinguish omitted MTF by more than twice the rounding allowance; difference={blur_difference}"
        );
        assert!(
            order_difference > 2. * TOLERANCE,
            "fixture must distinguish per-original display rendering from rendering the reconstructed mean; difference={order_difference}"
        );
    }
}
