use super::*;

#[test]
fn reduced_camera_proxies_reject_native_film_before_callback_construction() {
    use crate::{
        adjustment_layers::AdjustmentLayerSnapshot,
        film_emulation::FilmEmulationPreset,
        frame_raster::FrameRaster,
        geometry::Composition,
        gpu::{GpuAdjustmentLayers, SubmissionGate},
        gpu_scopes::{GpuScopes, ScopePaint},
        processing::InputSettings,
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let area = [120, 73, 1200, 1200];
        let mut image = GpuImage::new_shared(
            &renderer,
            &device,
            2,
            2,
            &[[12000, 16000, 18000, 0]; 4],
            DMat3::IDENTITY,
            OutputTarget::Srgb,
        )
        .await
        .unwrap();
        image
            .set_geometry(SourceGeometry::new(area, [2, 2]).unwrap())
            .unwrap();
        let grade = GpuAdjustmentLayers::new(
            &device,
            InputSettings::Digital,
            Default::default(),
            AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
            DMat3::IDENTITY,
            area.into(),
        )
        .unwrap();
        let clean = ProcessingInput::from(Arc::new(grade));
        let film = clean
            .clone()
            .with_film_emulation(FilmEmulationPreset::Medium.settings())
            .with_film_seed(17);
        image.require_processing(&clean).unwrap();
        assert!(image.require_processing(&film).is_err());
        let image = Arc::new(image);
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let composition =
            Composition::new(area.into(), Default::default(), Default::default()).unwrap();
        let frame = FrameRaster {
            output: [2, 2],
            ..FrameRaster::full(composition).unwrap()
        };
        let callback = |input| {
            ScopePaint::with_processing(
                gpu.clone(),
                image.clone(),
                input,
                frame,
                Default::default(),
                egui::Context::default(),
            )
        };
        assert!(callback(clean.clone()).is_ok());
        assert!(callback(film.clone()).is_err());
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &clean)
                .unwrap()
        );
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &film)
                .is_err()
        );
        assert!(
            !image
                .prepare_processing(&device, &mut encoder, &clean)
                .unwrap()
        );
        SubmissionGate::default().submit(&queue, [encoder.finish()]);

        let mut image = Arc::try_unwrap(image).ok().unwrap();
        image
            .set_geometry(SourceGeometry::new([120, 73, 2, 2], [2, 2]).unwrap())
            .unwrap();
        image.require_processing(&film).unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            image
                .prepare_processing(&device, &mut encoder, &film)
                .unwrap()
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    });
}
