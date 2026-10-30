use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings},
    geometry::{SourcePoint, SourceRegion},
};

#[test]
fn source_identity_layout_capacity_and_presentation_footprint_are_distinct() {
    use crate::{
        acquisition::Sampling,
        pixel_pipeline::{PixelPipeline, PixelStage},
    };
    let composition = Composition::new(
        [100, 80, 40, 30].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([5., 4.]),
        FramePoint([17., 10.]),
        [6, 3],
    )
    .unwrap();
    let base =
        ViewSourceSpec::new(7, frame, InputSettings::Digital, DMat3::IDENTITY, None).unwrap();
    let small = base
        .clone()
        .with_pipeline(PixelPipeline {
            stages: vec![PixelStage::new(crate::image_mtf::ImageMtfParameters {
                sigma: 2.,
            })],
        })
        .unwrap();
    let large = base
        .clone()
        .with_pipeline(PixelPipeline {
            stages: vec![PixelStage::new(crate::image_mtf::ImageMtfParameters {
                sigma: 3.,
            })],
        })
        .unwrap();
    assert!(small.same_source(&large));
    assert!(!small.same_acquisition(&large));
    assert!(large.satisfies(&small));
    assert!(!small.satisfies(&large));
    let finer = base
        .clone()
        .with_pipeline(PixelPipeline {
            stages: vec![PixelStage::new(crate::image_mtf::ImageMtfParameters {
                sigma: 0.6,
            })],
        })
        .unwrap();
    assert!(finer.plan.output().output[0] > large.plan.output().output[0]);
    assert!(
        !large.satisfies(&finer),
        "extra coarse neighbours cannot satisfy a finer sampling request"
    );
    assert!(
        !large.compatible_processing(&finer),
        "a coarse fallback must not run a finer effect request"
    );
    let moved = large
        .clone()
        .with_frame(FrameRaster {
            min: FramePoint([6., 4.]),
            max: FramePoint([18., 10.]),
            ..frame
        })
        .unwrap();
    assert!(large.same_source(&moved));
    assert!(
        large.compatible_processing(&moved),
        "the retained footprint can still be graded as a navigation fallback"
    );
    assert!(
        !large.satisfies(&moved),
        "fallback coverage is not the newly requested output"
    );
    let proxy = large
        .clone()
        .with_sampling(Sampling::CameraProxy {
            region: composition.area(),
            size: [10, 10],
        })
        .unwrap();
    assert!(large.same_source(&proxy));
    assert!(
        !large.compatible_processing(&proxy),
        "equal source identity does not substitute a different sample representation"
    );
    let other = FrameRaster::full(
        Composition::new(
            [101, 80, 40, 30].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        base.with_frame(other).is_err(),
        "changing a layout cannot relabel its captured source grid"
    );
}

#[test]
fn complete_grades_reuse_the_input_image_and_replace_all_layer_values() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let cal = calibration();
        let frame = frame(&scan);
        let source = ViewSource::acquire(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            None,
            ViewSourceSpec::new(7, frame, cal, scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        let neutral = read(&device, &queue, &source.pixels);
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let mut previous = None;
        for (global_ev, local_ev) in [(0.5, 0.75), (0.6, 0.75), (0.5, 0.5), (0.5, 0.)] {
            let snapshot = AdjustmentLayerSnapshot::new(vec![
                AdjustmentLayer::global(AdjustmentLayerSettings {
                    exposure: global_ev,
                    ..Default::default()
                }),
                AdjustmentLayer {
                    mask: AdjustmentLayerCoverage::Linear {
                        start: SourcePoint([0., 0.]),
                        end: SourcePoint([256., 192.]),
                    },
                    photo: AdjustmentLayerSettings {
                        exposure: local_ev,
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            ])
            .unwrap();
            let input = ProcessingInput::from(Arc::new(
                ProcessingInput::new(
                    &device,
                    cal,
                    RollSettings::default(),
                    snapshot,
                    scan.camera_to_ap1,
                    scan.area,
                )
                .unwrap(),
            ));
            let view = RenderedView::new(
                &device,
                source.clone(),
                &input,
                OutputTarget::Srgb,
                previous.as_ref(),
            )
            .unwrap();
            view.ensure_submitted(&device, &queue, &gate, &context);
            let graded = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
            for (i, (actual, original)) in graded.iter().zip(&neutral).enumerate() {
                let point = frame
                    .source_point([
                        (i as u32 % frame.output[0]) as f64 + 0.5,
                        (i as u32 / frame.output[0]) as f64 + 0.5,
                    ])
                    .0;
                let t = ((point[0] * 256. + point[1] * 192.) / (256_f64.powi(2) + 192_f64.powi(2)))
                    .clamp(0., 1.);
                let weight = t * t * (3. - 2. * t);
                let gain = (global_ev + local_ev * weight).exp2();
                for c in 0..3 {
                    let expected = f64::from(original[c]) * gain;
                    assert!(
                        (f64::from(actual[c]) - expected).abs() <= 2e-5 + 2e-5 * expected.abs(),
                        "{global_ev}/{local_ev} pixel{i} channel{c}: {} vs {expected}",
                        actual[c]
                    );
                }
                assert_eq!(actual[3], original[3]);
            }
            assert_eq!(neutral, read(&device, &queue, &source.pixels));
            previous = Some(view);
        }
    });
}

#[test]
fn spatial_grades_require_the_input_coordinate_extent_but_global_grades_do_not() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let cal = calibration();
        let source = ViewSource::acquire(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            None,
            ViewSourceSpec::new(7, frame(&scan), cal, scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        let local = AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([256., 192.]),
            },
            photo: AdjustmentLayerSettings {
                exposure: 0.5,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 1.,
        };
        let admit = |area, layer| {
            let pipeline = Arc::new(
                ProcessingInput::new(
                    &device,
                    cal,
                    RollSettings::default(),
                    AdjustmentLayerSnapshot::new(vec![layer]).unwrap(),
                    scan.camera_to_ap1,
                    area,
                )
                .unwrap(),
            );
            RenderedView::new(
                &device,
                source.clone(),
                &pipeline.into(),
                OutputTarget::Srgb,
                None,
            )
        };
        assert!(admit(scan.area, local).is_ok());
        for area in [
            SourceRegion {
                width: scan.area.width / 2,
                ..scan.area
            },
            SourceRegion {
                x: scan.area.x + 1,
                ..scan.area
            },
        ] {
            assert!(
                admit(area, local).is_err(),
                "local adjustments must not silently disappear outside a mismatched grade extent"
            );
            assert!(
                admit(area, AdjustmentLayer::global(local.photo)).is_ok(),
                "global grading does not depend on an input extent"
            );
        }
    });
}
