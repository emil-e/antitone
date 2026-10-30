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
            &device,
            InputSettings::Digital,
            roll,
            layers.clone(),
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
            a,
        )
        .unwrap(),
    );
    let input = ProcessingInput::from(gpu)
        .with_frame_size([620, 480])
        .with_film_seed(17)
        .with_film_emulation(crate::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.1,
            amount: 0.,
            colour: 0.5,
        });
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
        let mut order_difference = 0_f64;
        let mut blur_difference = 0_f64;
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
                            unfiltered[((p[1] - a.y) * a.width + p[0] - a.x) as usize].as_dvec3(),
                            *weight,
                        ))
                    }))
                    .unwrap();
                blur_difference = blur_difference.max((expected - without_mtf).abs().max_element());
                let working =
                    crate::sampling::weighted_mean_iter(cells.iter().map(|(p, weight)| {
                        Ok((
                            blurred[((p[1] - a.y) * a.width + p[0] - a.x) as usize],
                            *weight,
                        ))
                    }))
                    .unwrap();
                let reversed = cpu.render(&[working]).unwrap()[0].as_dvec3();
                order_difference = order_difference.max((expected - reversed).abs().max_element());
                let actual = glam::Vec3::from_array(
                    pixels[(y * frame.output[0] + x) as usize][..3]
                        .try_into()
                        .unwrap(),
                )
                .as_dvec3();
                assert!(
                    (expected - actual).abs().max_element() < TOLERANCE,
                    "{target:?} ({x},{y}) expected={expected:?} actual={actual:?}"
                );
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
