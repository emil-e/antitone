use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
};

#[test]
fn working_view_requires_the_complete_matching_layer_stack() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let cal = calibration();
        let roll = RollSettings::default();
        let spec = ViewSourceSpec::new(7, frame(&scan), cal, scan.camera_to_ap1, None).unwrap();
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
        let local = AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([256., 192.]),
            },
            photo: AdjustmentLayerSettings {
                exposure: 0.75,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 1.,
        };
        let photo = AdjustmentLayerSettings {
            exposure: 0.5,
            ..Default::default()
        };
        let snapshot = AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(photo),
            local,
        ])
        .unwrap();
        let input = ProcessingInput::from(Arc::new(
            crate::gpu::GpuAdjustmentLayers::new(
                &device,
                cal,
                roll,
                snapshot.clone(),
                scan.camera_to_ap1,
                scan.area,
            )
            .unwrap(),
        ));
        let global = crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(photo),
        ])
        .and_then(|snapshot| {
            crate::gpu::GpuAdjustmentLayers::new(
                &device,
                cal,
                roll,
                snapshot,
                scan.camera_to_ap1,
                scan.area,
            )
        })
        .map(Arc::new)
        .unwrap();
        let admit = |snapshot: AdjustmentLayerSnapshot, input: &ProcessingInput| {
            RenderedView::new(
                &device,
                source.clone(),
                cal,
                roll,
                snapshot,
                input,
                OutputTarget::Srgb,
                None,
            )
        };
        assert!(admit(snapshot.clone(), &input).is_ok());
        let wrong_global = AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
                exposure: 0.6,
                ..photo
            }),
            local,
        ])
        .unwrap();
        assert!(admit(wrong_global, &input).is_err());
        let wrong_local = AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(photo),
            AdjustmentLayer {
                photo: AdjustmentLayerSettings {
                    exposure: 0.5,
                    ..local.photo
                },
                ..local
            },
        ])
        .unwrap();
        assert!(admit(wrong_local, &input).is_err());
        assert!(admit(snapshot, &ProcessingInput::from(global)).is_err());
    });
}
