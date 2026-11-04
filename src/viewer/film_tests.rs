use super::*;
use crate::gpu::source::SourceGeometry;

#[test]
fn pixel_processing_requires_declared_geometry_not_a_particular_preceding_representation() {
    use crate::{
        adjustment_layers::AdjustmentLayerSnapshot,
        frame_raster::FrameRaster,
        geometry::Composition,
        gpu::test_inputs::AuthoredPhotograph,
        gpu_scopes::{GpuScopes, ScopePaint},
        grain::GrainPreset,
        processing::InputSettings,
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let area = [120, 73, 1200, 1200];
        let image = GpuImage::new_shared(
            &renderer,
            &device,
            [2, 2],
            &[[12000, 16000, 18000, 0]; 4],
            DMat3::IDENTITY,
            None,
            OutputTarget::Srgb,
        )
        .await
        .unwrap();
        let grade = AuthoredPhotograph::new(
            &device.limits(),
            InputSettings::Digital,
            Default::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            DMat3::IDENTITY,
            area.into(),
        )
        .unwrap();
        let clean = AuthoredPhotograph::from(Arc::new(grade));
        let film = clean.clone().with_pixel_pipeline(
            crate::pixel_pipeline::PixelPipeline::emulation(
                GrainPreset::ThirtyFiveMm.settings(),
                Default::default(),
                [1200; 2],
                17,
            )
            .unwrap(),
        );
        image.require_processing(&clean).unwrap();
        assert!(
            image.require_processing(&film).is_err(),
            "coordinate-dependent stages require geometry"
        );
        let image = Arc::new(image);
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let composition =
            Composition::new(area.into(), Default::default(), Default::default()).unwrap();
        let frame = FrameRaster {
            output: [2, 2],
            ..FrameRaster::full(composition).unwrap()
        };
        assert!(
            ScopePaint::with_processing(
                gpu,
                image.clone(),
                film.clone(),
                frame,
                Default::default(),
                egui::Context::default()
            )
            .is_err()
        );
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
        queue.submit([encoder.finish()]);
        drop(image);
        for region in [[120, 73, 8, 8], [120, 73, 2, 2]] {
            let image = GpuImage::new_shared(
                &renderer,
                &device,
                [2, 2],
                &[[12000, 16000, 18000, 0]; 4],
                DMat3::IDENTITY,
                Some(SourceGeometry::new(region, [2, 2]).unwrap()),
                OutputTarget::Srgb,
            )
            .await
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
            assert!(
                crate::viewer::readback::image_pixels(&device, &queue, &image)
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite())
            );
        }
    });
}

#[test]
fn cropped_frame_not_grade_source_or_tile_dimensions_controls_film_scale() {
    use crate::adjustment_layers::AdjustmentLayer;
    use crate::adjustment_layers::AdjustmentLayerSettings;
    use crate::adjustment_layers::AdjustmentLayerSnapshot;
    use crate::gpu::grading::Grading;
    use crate::gpu::interpretation::InputTransform;
    use crate::gpu::test_inputs::AuthoredPhotograph;
    use crate::processing::InputSettings;
    use crate::processing::RollSettings;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let small = [0, 0, 6000, 3000].into();
        let large = [0, 0, 6000, 6000].into();
        let roll = RollSettings {
            exposure: 0.5,
            ..Default::default()
        };
        let snapshot =
            AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
                exposure: 0.25,
                ..Default::default()
            })])
            .unwrap();
        let shared =
            Arc::new(Grading::new(&device.limits(), roll, snapshot.clone(), small).unwrap());
        let fresh = Arc::new(Grading::new(&device.limits(), roll, snapshot, large).unwrap());
        let image = GpuImage::new_continuous_shared(
            &renderer,
            &device,
            [64, 64],
            &vec![[6000., 10000., 14000., 0.]; 64 * 64],
            DMat3::IDENTITY,
            Some(SourceGeometry::new([0, 0, 64, 64], [64, 64]).unwrap()),
            OutputTarget::Srgb,
        )
        .await
        .unwrap();
        let render = |grade, area, frame_size| {
            let pipeline = AuthoredPhotograph::compose(
                &device.limits(),
                InputTransform::new(InputSettings::Digital, DMat3::IDENTITY).unwrap(),
                grade,
                area,
            )
            .unwrap();
            let input = AuthoredPhotograph::from(Arc::new(pipeline)).with_pixel_pipeline(
                crate::pixel_pipeline::PixelPipeline::emulation(
                    crate::grain::GrainSettings {
                        enabled: true,
                        size: 0.065,
                        image_sigma: 0.065,
                        amount: 0.065,
                        ..Default::default()
                    },
                    Default::default(),
                    frame_size,
                    17,
                )
                .unwrap(),
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            image
                .encode_processing(&device, &mut encoder, &input)
                .unwrap();
            queue.submit([encoder.finish()]);
            crate::viewer::readback::image_pixels(&device, &queue, &image)
        };
        let expected = render(fresh, large, [3200, 2400]);
        let actual = render(shared.clone(), large, [3200, 2400]);
        let other_source = render(shared.clone(), small, [3200, 2400]);
        let other_frame = render(shared, large, [3200, 1200]);
        let difference = |a: &[[f32; 4]], b: &[[f32; 4]]| {
            a.iter()
                .zip(b)
                .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
                .fold(0_f32, f32::max)
        };
        assert!(
            difference(&expected, &actual) < 1e-5,
            "reusing a global grade must retain the captured crop's film scale"
        );
        assert!(
            difference(&expected, &other_source) < 1e-5,
            "uncropped source dimensions must not redefine a captured crop's film scale"
        );
        assert!(
            difference(&expected, &other_frame) > 0.001,
            "the image must distinguish the two source-frame grain scales"
        );
    });
}
