//! Identical canonical AP1 inputs exercise the actual viewer and export repair pipelines.
use super::*;
use crate::{
    dust_repair::{DonorChoice, Repair, RepairId, RepairOrigin, SupportMask},
    gpu::{ProcessingSource, flat::SourceGeometry},
    processing::InvertedPixel,
};

#[test]
fn shared_colour_preserves_signed_wb_and_luminance_cancellation_before_local_recovery() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let wb = Balance {
            warmth: 0.35,
            tint: -0.2,
        };
        for (label, row, global, first) in [
            (
                "local-wb",
                crate::color::wb_matrix(wb).unwrap().row(0),
                PhotoSettings::default(),
                PhotoSettings {
                    wb,
                    ..Default::default()
                },
            ),
            (
                "local-wb-fractional-exposure",
                crate::color::wb_matrix(wb).unwrap().row(0),
                PhotoSettings::default(),
                PhotoSettings {
                    wb,
                    exposure: 0.25,
                    ..Default::default()
                },
            ),
            (
                "global-wb",
                crate::color::wb_matrix(wb).unwrap().row(0),
                PhotoSettings {
                    wb,
                    ..Default::default()
                },
                PhotoSettings::default(),
            ),
            (
                "global-wb-fractional-exposure",
                crate::color::wb_matrix(wb).unwrap().row(0),
                PhotoSettings {
                    wb,
                    exposure: 0.25,
                    ..Default::default()
                },
                PhotoSettings::default(),
            ),
            (
                "luminance",
                crate::color::AP1_TO_XYZ.row(1),
                PhotoSettings {
                    saturation: 0.,
                    ..Default::default()
                },
                PhotoSettings::default(),
            ),
        ] {
            let mut values = Vec::new();
            for r in [0.25, 1., 16.] {
                let b = 0.004;
                let g = (-(row.x * r + row.z * b) / row.y) as f32;
                for offset in -2..=2 {
                    values.push([
                        r as f32,
                        f32::from_bits(g.to_bits().checked_add_signed(offset).unwrap()),
                        b as f32,
                        9.,
                    ]);
                }
            }
            for start in [11, (1 << 20) + 1] {
                let area: SourceRegion = [start, start, 256, 128].into();
                let region: SourceRegion = [start + 64, start + 48, values.len() as u32, 1].into();
                let composition = crate::geometry::Composition::with_crop(
                    area,
                    Default::default(),
                    Default::default(),
                    Some(crate::crop::Crop {
                        center: [
                            region.x as f64 + region.width as f64 * 0.5,
                            region.y as f64 + 0.5,
                        ],
                        size: [region.width, 1],
                        angle: 0.,
                    }),
                )
                .unwrap();
                let frame = FrameRaster::full(composition).unwrap();
                let full = |photo| LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([start as f64 - 1., start as f64]),
                        end: SourcePoint([start as f64, start as f64]),
                    },
                    photo,
                    enabled: true,
                    invert: false,
                    strength: 1.,
                };
                let snapshot = PhotoGradeSnapshot::new(
                    global,
                    vec![
                        full(first),
                        full(PhotoSettings {
                            exposure: 12.,
                            ..Default::default()
                        }),
                        LocalAdjustment {
                            mask: GradientMask::Linear {
                                start: SourcePoint([region.x as f64, region.y as f64]),
                                end: SourcePoint([
                                    (region.x + region.width) as f64,
                                    region.y as f64,
                                ]),
                            },
                            photo: PhotoSettings {
                                exposure: 0.25,
                                ..Default::default()
                            },
                            enabled: true,
                            invert: false,
                            strength: 0.7,
                        },
                    ],
                )
                .unwrap();
                let source =
                    authored_source(&device, renderer.clone(), kernels.clone(), &values, frame);
                let roll = RollSettings {
                    gamma: 0.65,
                    ..RollSettings::new_grade()
                };
                let scalar = crate::local_adjustments::SpatialPipeline::new(
                    calibration(),
                    roll,
                    snapshot.clone(),
                    source.spec.matrix,
                )
                .unwrap();
                let view = render(
                    &device,
                    &queue,
                    source.clone(),
                    snapshot.clone(),
                    OutputTarget::Srgb,
                );
                let viewed = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
                let grade = Arc::new(
                    GpuPhotoGrade::new(
                        &device,
                        calibration(),
                        roll,
                        snapshot,
                        source.spec.matrix,
                        area,
                    )
                    .unwrap(),
                );
                let patches = values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| {
                        let repair = Repair {
                            id: RepairId(i as u64 + 1),
                            enabled: true,
                            origin: RepairOrigin::Manual,
                            support: SupportMask::new(
                                [region.x + i as u32, region.y],
                                [1, 1],
                                vec![true],
                            )
                            .unwrap(),
                            donor: DonorChoice::Offset([0, 32]),
                        };
                        let patch = crate::dust_repair::evaluate(&repair, area, |_| {
                            Ok([value[0], value[1], value[2]])
                        })
                        .unwrap();
                        assert!(patch.pixels().all(|(_, rgb)| rgb == value[..3]));
                        Arc::new(patch)
                    })
                    .collect::<Vec<_>>();
                let mut input = ProcessingInput::from(grade.clone());
                input.repairs = crate::gpu::repairs::Snapshot::logical(patches).unwrap();
                GpuPhotoGrade::validate_repaired(
                    &device,
                    calibration(),
                    roll,
                    grade.snapshot(),
                    source.spec.matrix,
                    area,
                    input.repairs.bound(),
                )
                .unwrap();
                let geometry = SourceGeometry::original_tile(
                    [region.x, region.y, region.width, region.height],
                    [region.width, 1],
                )
                .unwrap();
                let mut originals = ProcessingSource::new_continuous(
                    &device,
                    &vec![[0., 20000., 25000., 0.]; values.len()],
                    source.spec.matrix,
                )
                .unwrap();
                originals.set_flat_field(None, geometry).unwrap();
                let working = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: values.len() as u64 * 16,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let kernel = renderer.export_kernel(&device).unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                kernel
                    .encode_grade(&device, &mut encoder, &originals, &working, &grade)
                    .unwrap();
                renderer
                    .repairs
                    .original(&device, &mut encoder, kernel, &working, &input, geometry)
                    .unwrap();
                let exported = crate::gpu::arithmetic_research::tests::read_working(
                    &device, &queue, encoder, &working,
                );
                let mut largest_signal = 0_f64;
                for (i, ((value, actual), preview)) in
                    values.iter().zip(exported).zip(viewed).enumerate()
                {
                    let point = [region.x + i as u32, region.y];
                    let neutral = InvertedPixel {
                        rgb: DVec3::new(value[0] as f64, value[1] as f64, value[2] as f64),
                        floored: [true, false, false],
                    };
                    let expected = scalar.grade_neutral(neutral, point).unwrap();
                    largest_signal = largest_signal.max(expected.rgb.x.abs());
                    for (path, result, flags) in [("export", actual, 1.), ("viewer", preview, 9.)] {
                        for (c, &component) in result[..3].iter().enumerate() {
                            let error = (f64::from(component) - expected.rgb[c]).abs();
                            assert!(
                                component.is_finite()
                                    && error <= 2e-5 + 2e-4 * expected.rgb[c].abs(),
                                "{label} {path} origin={start} pixel={i} c={c} actual={component} expected={} error={error}",
                                expected.rgb[c]
                            );
                        }
                        assert_eq!(
                            result[3], flags,
                            "preserve original floor/coverage flags after repair and recovery"
                        );
                    }
                }
                assert!(
                    largest_signal > 2e-5,
                    "neighboring represented colours must retain a recovered cancellation signal"
                );
                assert_eq!(
                    read(&device, &queue, &source.pixels),
                    values,
                    "immutable canonical input"
                );
            }
        }
    });
}
