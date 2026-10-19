use super::*;

#[test]
fn low_gamma_balanced_floor_anchor_survives_camera_cancellation() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let baseline = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let candidate = renderer(&device, &queue, SubmissionGate::default())
            .await
            .unwrap();
        for scale in [
            1., 1.01, 1.1, 1.2, 1.5, 2., 2.1, 3., 4., 5., 8., 16., 32., 64., 128.,
        ] {
            let floor = 1.816128786144808e-5 * scale;
            let calibration = Calibration {
                dmin: DVec3::splat(4.104451056687266e-5 * scale),
                dmax: DVec3::splat(floor),
                input_floor: floor,
            };
            let roll = RollSettings {
                gamma: 0.1,
                balance: crate::color::Balance {
                    warmth: -1.,
                    tint: -1.,
                },
                response: crate::processing::Response::Film {
                    toe: 1.,
                    shoulder: 0.02,
                },
                exposure: -1.,
                ..RollSettings::new_grade()
            };
            let matrix = DMat3::from_cols(DVec3::X, DVec3::new(-1., 1., 0.), DVec3::Z);
            let area = crate::geometry::SourceRegion::from([0, 0, 1, 1]);
            let grade = GpuPhotoGrade::new(
                &device,
                calibration,
                roll,
                crate::local_adjustments::PhotoGradeSnapshot::new(Default::default(), vec![])
                    .unwrap(),
                matrix,
                area,
            )
            .unwrap();
            let source = ProcessingSource::new_continuous(&device, &[[0.; 4]], matrix).unwrap();
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            for (name, kernel) in [
                ("current", &baseline.kernel),
                ("candidate", &candidate.kernel),
            ] {
                let mut encoder = device.create_command_encoder(&Default::default());
                kernel
                    .encode_grade(&device, &mut encoder, &source, &output, &grade)
                    .unwrap();
                let actual = tests::read_working(&device, &queue, encoder, &output)[0];
                println!("export_density_anchor scale={scale} variant={name} actual={actual:?}");
                // Each channel is exactly at Dmax after flooring, so exposure gives 1/2
                // independently of its gamma. The camera's red difference must remain zero.
                for (c, expected) in [0_f64, 0.5, 0.5].into_iter().enumerate() {
                    assert!(
                        (f64::from(actual[c]) - expected).abs() <= 2e-5 + 2e-4 * expected.abs(),
                        "{name}, channel {c}: {actual:?}"
                    );
                }
                assert_eq!(actual[3], 7.);
            }
        }
    });
}
