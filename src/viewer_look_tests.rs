use super::*;
#[test]
fn idle_host_polling_completes_projected_look_admission_without_another_frame_or_submission() {
    use crate::{
        gpu::SubmissionGate,
        looks::{LookAsset, LookOrigin, Lut3D},
        processing::{Calibration, PhotoSettings, RollSettings},
    };
    use glam::DVec3;
    use std::time::{Duration, Instant};
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let gate = SubmissionGate::default();
        let renderer = GpuRenderer::new_with_submissions(&device, &queue, gate.clone())
            .await
            .unwrap();
        let image = GpuImage::new_shared(
            &renderer,
            &device,
            2,
            2,
            &[[32000, 32000, 32000, 0]; 4],
            DMat3::IDENTITY,
            OutputTarget::Srgb,
        )
        .await
        .unwrap();
        let look = Arc::new(
            LookAsset::new(
                "Bounded input".into(),
                LookOrigin::Imported,
                Arc::new(Lut3D::new(2, [0.99; 3], [1.; 3], vec![[0.4; 3]; 8]).unwrap()),
            )
            .unwrap(),
        );
        let input = ProcessingInput::from(
            ProcessingParameters::positive(
                Calibration {
                    dmin: DVec3::splat(0.7),
                    dmax: DVec3::splat(0.003),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                RollSettings::default(),
                PhotoSettings::default(),
                DMat3::IDENTITY,
            )
            .unwrap(),
        )
        .with_look(Some(look));
        let mut encoder = device.create_command_encoder(&Default::default());
        image
            .prepare_rendering(
                &device,
                &queue,
                &mut encoder,
                &input,
                TextureMapping::IDENTITY,
                None,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        // The native host polls only tracked work while idle. No further UI frame,
        // queue submission, or unconditional blocking wait delivers this callback.
        while image.look_result_for(TextureMapping::IDENTITY).is_none() {
            assert!(gate.pending(), "readback would strand in the idle host");
            device.poll(wgpu::PollType::Poll).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            image
                .look_result_for(TextureMapping::IDENTITY)
                .unwrap()
                .is_ok()
        );
        assert!(!gate.pending());
    });
}
