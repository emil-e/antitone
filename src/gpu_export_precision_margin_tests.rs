use super::*;
use crate::{
    geometry::SourceRegion,
    local_adjustments::{PhotoGradeSnapshot, SpatialPipeline},
    processing::{Inversion, PhotoSettings, Response},
};

#[test]
fn independent_continuous_inputs_cover_dense_camera_cancellation_and_response_margins() {
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
        let area = SourceRegion::from([27, 43, 1024, 1]);
        let pivot = [100.125_f32, 125.375, 210.5, 0.];
        let pixels: Vec<[f32; 4]> = (0..area.width)
            .map(|i| match i {
                0 => [0.; 4],
                1 => pivot,
                2 => [1000.125, 5000.375, 9000.5, 0.],
                _ => {
                    let d = (i % 127) as f32 / 32.;
                    if i >= 900 {
                        [
                            70000.125 + d * 1000.,
                            120000.375 + d * 2000.,
                            90000.5 + d * 3000.,
                            0.,
                        ]
                    } else if i < 512 {
                        [pivot[0] + d, pivot[1] + d, pivot[2] + d, 0.]
                    } else {
                        [
                            1000.125 + d * 120.,
                            5000.375 + d * 150.,
                            9000.5 + d * 500.,
                            0.,
                        ]
                    }
                }
            })
            .collect();
        let calibration = Calibration {
            dmin: DVec3::new(0.6, 0.4, 0.2),
            dmax: DVec3::new(0.01, 0.008, 0.003),
            input_floor: 1. / 65535.,
        };
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: pixels.len() as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        for (response, gamma) in [
            (Response::film(), 0.6),
            (
                Response::Film {
                    toe: 4.,
                    shoulder: 0.01,
                },
                0.4,
            ),
            (
                Response::Film {
                    toe: 4.,
                    shoulder: 0.,
                },
                0.6,
            ),
            (
                Response::Film {
                    toe: 3.,
                    shoulder: 0.01,
                },
                1.2,
            ),
        ] {
            let roll = RollSettings {
                response,
                gamma,
                ..RollSettings::new_grade()
            };
            let sample = DVec3::new(
                f64::from(pivot[0]),
                f64::from(pivot[1]),
                f64::from(pivot[2]),
            ) / 65535.;
            let native = Inversion::new(calibration, roll)
                .unwrap()
                .apply(sample)
                .unwrap()
                .rgb;
            // This admitted matrix cancels one independently evaluated camera-positive pair
            // at the pivot, exposing errors hidden by relative gates on large native values.
            let cancellation =
                DMat3::from_cols(DVec3::X, DVec3::new(-native.x / native.y, 1., 0.), DVec3::Z);
            let camera = DMat3::from_cols(
                DVec3::new(0.876717558, -0.027155151, 0.001892225),
                DVec3::new(0.301847779, 1.388610987, -0.152684962),
                DVec3::new(-0.178565331, -0.361455645, 1.150792743),
            );
            let middle = Inversion::new(calibration, roll)
                .unwrap()
                .apply(DVec3::new(1000.125, 5000.375, 9000.5) / 65535.)
                .unwrap()
                .rgb;
            let low_cancellation =
                DMat3::from_cols(DVec3::X, DVec3::new(-middle.x / middle.y, 1., 0.), DVec3::Z);
            for (label, matrix) in [
                ("dense-cancellation", cancellation),
                ("low-response-cancellation", low_cancellation),
                ("camera", camera),
            ] {
                for (exposure, local_ev) in [(0., 0.), (6., 0.), (16., 0.), (0., 12.)] {
                    let snapshot = PhotoGradeSnapshot::new(
                        PhotoSettings {
                            exposure,
                            ..Default::default()
                        },
                        if local_ev == 0. {
                            vec![]
                        } else {
                            vec![crate::local_adjustments::LocalAdjustment {
                                mask: crate::local_adjustments::GradientMask::Linear {
                                    start: crate::geometry::SourcePoint([0., 0.]),
                                    end: crate::geometry::SourcePoint([1., 0.]),
                                },
                                photo: PhotoSettings {
                                    exposure: local_ev,
                                    ..Default::default()
                                },
                                enabled: true,
                                invert: false,
                                strength: 1.,
                            }]
                        },
                    )
                    .unwrap();
                    let grade = GpuPhotoGrade::new(
                        &device,
                        calibration,
                        roll,
                        snapshot.clone(),
                        matrix,
                        area,
                    )
                    .unwrap();
                    let scalar = SpatialPipeline::new(calibration, roll, snapshot, matrix).unwrap();
                    let mut source =
                        ProcessingSource::new_continuous(&device, &pixels, matrix).unwrap();
                    source
                        .set_geometry(
                            SourceGeometry::original_tile(
                                [area.x, area.y, area.width, area.height],
                                [area.width, area.height],
                            )
                            .unwrap(),
                        )
                        .unwrap();
                    for (variant, kernel) in [
                        ("current", &baseline.kernel),
                        ("candidate", &candidate.kernel),
                    ] {
                        let mut encoder = device.create_command_encoder(&Default::default());
                        kernel
                            .encode_grade(&device, &mut encoder, &source, &output, &grade)
                            .unwrap();
                        let actual = tests::read_working(&device, &queue, encoder, &output);
                        let mut failures = 0;
                        let mut worst = 0_f64;
                        let mut witness = None;
                        for (i, p) in actual.iter().enumerate() {
                            let source = DVec3::new(
                                f64::from(pixels[i][0]),
                                f64::from(pixels[i][1]),
                                f64::from(pixels[i][2]),
                            ) / 65535.;
                            let expected =
                                scalar.apply(source, [area.x + i as u32, area.y]).unwrap();
                            let flags = expected
                                .floored
                                .iter()
                                .enumerate()
                                .fold(0u32, |m, (c, &floor)| m | (u32::from(floor) << c));
                            assert_eq!(p[3], flags as f32);
                            for (c, &value) in p[..3].iter().enumerate() {
                                let error = (f64::from(value) - expected.rgb[c]).abs();
                                assert!(value.is_finite());
                                worst = worst.max(error);
                                if error > 2e-5 + 2e-4 * expected.rgb[c].abs() {
                                    failures += 1;
                                    witness.get_or_insert((i, c, value, expected.rgb[c], error));
                                }
                            }
                        }
                        println!(
                            "export_margin matrix={label} response={response:?} gamma={gamma} exposure={exposure} local_ev={local_ev} variant={variant} violations={failures} max_absolute={worst:.9} first={witness:?}"
                        );
                        assert_eq!(
                            failures, 0,
                            "independent working gate: {variant} {label} {response:?} EV {exposure} local EV {local_ev}"
                        );
                    }
                }
            }
        }
    });
}
