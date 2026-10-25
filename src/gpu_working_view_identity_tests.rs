use super::*;
use crate::{
    geometry::SourcePoint,
    local_adjustments::{GradientMask, LocalAdjustment},
};

#[test]
fn local_precision_budget_is_not_global_identity_and_mismatched_grades_still_reject() {
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
        let local = LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([256., 192.]),
            },
            photo: PhotoSettings {
                exposure: 0.75,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 1.,
        };
        let photo = PhotoSettings {
            exposure: 0.5,
            ..Default::default()
        };
        let snapshot = PhotoGradeSnapshot::new(photo, vec![local]).unwrap();
        let input = ProcessingInput::from(Arc::new(
            crate::gpu::GpuPhotoGrade::new(
                &device,
                cal,
                roll,
                snapshot.clone(),
                scan.camera_to_ap1,
                scan.area,
            )
            .unwrap(),
        ));
        let global = ProcessingParameters::positive(cal, roll, photo, scan.camera_to_ap1).unwrap();
        assert!(
            input.parameters().response_precision.x < global.response_precision.x,
            "the fixture must exercise an active local precision adjustment"
        );
        let admit = |snapshot: PhotoGradeSnapshot, input: &ProcessingInput| {
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
        let wrong_global = PhotoGradeSnapshot::new(
            PhotoSettings {
                exposure: 0.6,
                ..photo
            },
            vec![local],
        )
        .unwrap();
        assert!(admit(wrong_global, &input).is_err());
        let wrong_local = PhotoGradeSnapshot::new(
            photo,
            vec![LocalAdjustment {
                photo: PhotoSettings {
                    exposure: 0.5,
                    ..local.photo
                },
                ..local
            }],
        )
        .unwrap();
        assert!(admit(wrong_local, &input).is_err());
        assert!(admit(snapshot, &ProcessingInput::from(global)).is_err());
    });
}
