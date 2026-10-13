use super::*;
use antitone::{
    color::Balance,
    gpu::{GpuFlatField, SourceGeometry},
    processing::{Calibration, PhotoSettings, RollSettings},
    renderer::GpuRenderer,
    viewer::ImagePaint,
};
use glam::DVec3;

#[test]
fn shared_renderer_keeps_scope_viewer_and_background_image_state_independent() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [600, 400],
            pixels_per_point: 2.,
        };
        let mut resources = egui_wgpu::CallbackResources::default();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            for viewer_first in [false, true] {
                let field = Arc::new(
                    antitone::flat_field::FlatField::from_nodes(
                        [12, 7, 8, 4],
                        [2, 2],
                        vec![DVec3::new(1.1, 0.9, 1.2); 4],
                    )
                    .unwrap(),
                );
                let calibration = field
                    .processing_calibration(Calibration {
                        dmin: DVec3::new(0.8, 0.7, 0.9),
                        dmax: DVec3::new(0.003, 0.002, 0.001),
                        input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
                    })
                    .unwrap();
                let parameters = ProcessingParameters::positive(
                    calibration,
                    RollSettings::new_grade(),
                    PhotoSettings::default(),
                    DMat3::IDENTITY,
                )
                .unwrap();
                let pixels: Vec<_> = (0..32u16)
                    .map(|i| [2000 + i * 900, 3000 + i * 700, 4000 + i * 1100, 0])
                    .collect();
                let mut image = GpuImage::new_shared(
                    &renderer,
                    &device,
                    8,
                    4,
                    &pixels,
                    DMat3::IDENTITY,
                    target,
                )
                .await
                .unwrap();
                let revision = image.source_revision();
                image
                    .set_flat_field(
                        Some(Arc::new(GpuFlatField::new(&device, field).unwrap())),
                        SourceGeometry::new([12, 7, 8, 4], [8, 4]).unwrap(),
                    )
                    .unwrap();
                assert_eq!(image.source_revision(), revision + 1);
                let image = Arc::new(image);
                let frame = full_frame([8, 4]);
                let settings = Settings {
                    target,
                    plot: Plot::Vector,
                    ..Default::default()
                };
                let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
                let scope = ScopePaint::new(
                    gpu.clone(),
                    image.clone(),
                    parameters,
                    frame,
                    settings,
                    egui::Context::default(),
                );
                let viewer = ImagePaint::new(image.clone(), Some(parameters), [0.25, 0., 0.5, 1.]);
                let mut encoder = device.create_command_encoder(&Default::default());
                if viewer_first {
                    viewer.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                    assert!(!image.prepare(&device, &mut encoder, parameters));
                    scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                } else {
                    scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                    assert!(!image.prepare(&device, &mut encoder, parameters));
                    viewer.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                }
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let counts = gpu
                    .status(&image, parameters, frame, settings)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    counts.valid, 32,
                    "viewer crop must not narrow scope coverage"
                );
                let bins = read(&device, &queue, &gpu.bins);

                // Different matrices, source storage and parameters still share only pipelines.
                let thumbnail = Arc::new(
                    GpuImage::new_shared(
                        &renderer,
                        &device,
                        4,
                        2,
                        &[[42000, 12000, 6000, 0]; 8],
                        DMat3::from_diagonal(DVec3::new(0.8, 1.1, 0.9)),
                        target,
                    )
                    .await
                    .unwrap(),
                );
                let tile = GpuImage::new_shared(
                    &renderer,
                    &device,
                    8,
                    4,
                    &[[17000, 24000, 31000, 0]; 32],
                    DMat3::from_diagonal(DVec3::new(1.2, 0.7, 1.1)),
                    target,
                )
                .await
                .unwrap();
                let tile_parameters = ProcessingParameters::positive(
                    calibration,
                    RollSettings::new_grade(),
                    PhotoSettings {
                        wb: Balance {
                            warmth: 0.2,
                            tint: -0.1,
                        },
                        exposure: -1.,
                        ..PhotoSettings::default()
                    },
                    DMat3::IDENTITY,
                )
                .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                assert!(thumbnail.prepare(&device, &mut encoder, ProcessingParameters::source()));
                tile.encode(&device, &mut encoder, tile_parameters);
                scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                assert!(!image.prepare(&device, &mut encoder, parameters));
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                assert_eq!(gpu.reduction_count(), 1);
                assert_eq!(
                    gpu.status(&image, parameters, frame, settings)
                        .unwrap()
                        .unwrap(),
                    counts
                );
                assert_eq!(read(&device, &queue, &gpu.bins), bins);
                assert!(
                    gpu.status(&thumbnail, parameters, frame, settings)
                        .is_none(),
                    "renderer identity must not substitute for image identity"
                );

                let revision = image.source_revision();
                assert!(image.write_source(&queue, &pixels[..31]).is_err());
                assert_eq!(image.source_revision(), revision);
                assert!(gpu.status(&image, parameters, frame, settings).is_some());
                image
                    .write_source(&queue, &[[8000, 16000, 24000, 0]; 32])
                    .unwrap();
                assert_eq!(image.source_revision(), revision + 1);
                assert!(gpu.status(&image, parameters, frame, settings).is_none());
                let mut encoder = device.create_command_encoder(&Default::default());
                scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                viewer.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                assert!(!image.prepare(&device, &mut encoder, parameters));
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                assert_eq!(gpu.reduction_count(), 2);
                assert_ne!(read(&device, &queue, &gpu.bins), bins);
            }
        }
    });
}
