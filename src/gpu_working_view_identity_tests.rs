use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings},
    geometry::{SourcePoint, SourceRegion},
};

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
                GpuPipeline::new(
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
                GpuPipeline::new(
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
