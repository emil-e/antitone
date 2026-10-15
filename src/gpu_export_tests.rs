use super::*;
use crate::{
    crop::Crop,
    geometry::{Flips, PhotoOrientation, QuarterTurns, RollOrientation, SourceRegion},
};

pub(super) fn context() -> (wgpu::Device, wgpu::Queue, Arc<GpuRenderer>) {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    })
}
fn codes(point: [u32; 2]) -> [u16; 4] {
    let [x, y] = point;
    match (x + y) % 13 {
        0 => [0; 4],
        1 => [65535; 4],
        _ => [
            x.wrapping_mul(1871) as u16,
            y.wrapping_mul(7919) as u16,
            (x + y).wrapping_mul(3413) as u16,
            0,
        ],
    }
}
fn tile(origin: [u32; 2], size: [u32; 2]) -> Vec<[u16; 4]> {
    (0..size[0] * size[1])
        .map(|i| codes([origin[0] + i % size[0], origin[1] + i / size[0]]))
        .collect()
}

#[cfg(feature = "ocio-reference")]
#[test]
fn original_tile_math_and_aces_preserve_existing_independent_gates() {
    use crate::{
        color::Balance,
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment, SpatialPipeline},
        processing::PhotoSettings,
        rendering::CpuRenderer,
    };
    let (device, queue, renderer) = context();
    let submissions = SubmissionGate::default();
    let area = SourceRegion::from([10, 20, 513, 65]);
    let matrix = DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    );
    let calibration = Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::new_grade();
    let global = PhotoSettings {
        contrast: 0.8,
        saturation: 1.3,
        wb: Balance {
            warmth: 0.01,
            tint: -0.005,
        },
        ..Default::default()
    };
    let locals = vec![
        LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([10., 20.]),
                end: SourcePoint([523., 85.]),
            },
            photo: PhotoSettings {
                exposure: 0.5,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.7,
        },
        LocalAdjustment {
            mask: GradientMask::Radial {
                center: SourcePoint([270., 52.]),
                radii: [200., 26.],
                angle: 0.3,
                feather: 0.,
            },
            photo: PhotoSettings {
                saturation: 1.1,
                ..Default::default()
            },
            enabled: true,
            invert: true,
            strength: 1.,
        },
    ];
    for corrected in [false, true] {
        let field = corrected.then(|| {
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [10, 20, 513, 65],
                    [2, 2],
                    vec![
                        DVec3::new(0.9, 1.1, 1.),
                        DVec3::new(1.2, 0.8, 1.1),
                        DVec3::new(0.95, 1.2, 0.85),
                        DVec3::new(1.1, 0.95, 1.2),
                    ],
                )
                .unwrap(),
            )
        });
        let calibration = field
            .as_ref()
            .map_or(Ok(calibration), |f| f.processing_calibration(calibration))
            .unwrap();
        for locals in [Vec::new(), locals.clone()] {
            let snapshot = PhotoGradeSnapshot::new(global, locals).unwrap();
            let scalar = SpatialPipeline::new(calibration, roll, snapshot.clone(), matrix).unwrap();
            for format in [
                Format::Exr(ExrPrecision::Full),
                Format::Tiff(OutputTarget::Srgb),
                Format::Tiff(OutputTarget::DisplayP3),
            ] {
                let grade =
                    GpuPhotoGrade::new(&device, calibration, roll, snapshot.clone(), matrix, area)
                        .unwrap();
                let field_gpu = field
                    .as_ref()
                    .map(|f| Arc::new(GpuFlatField::new(&device, f.clone()).unwrap()));
                let mut processor = OriginalTiles::new(
                    &renderer,
                    (&device, &queue, &submissions),
                    (matrix, area),
                    field_gpu,
                    grade,
                    format,
                )
                .unwrap();
                for origin in [[10, 20], [266, 20], [395, 20]] {
                    let pixels = tile(origin, tile_size(area));
                    let actual = processor
                        .read(&renderer, &device, &queue, &submissions, origin, &pixels)
                        .unwrap();
                    let expected: Vec<_> = pixels
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            let point = [
                                origin[0] + i as u32 % TILE[0],
                                origin[1] + i as u32 / TILE[0],
                            ];
                            let gain = field.as_ref().map_or(DVec3::ONE, |f| {
                                f.gain(point.map(|v| v as f64 + 0.5)).unwrap()
                            });
                            scalar
                                .apply(
                                    DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.
                                        * gain,
                                    point,
                                )
                                .unwrap()
                        })
                        .collect();
                    let rendered = match format {
                        Format::Tiff(target) => Some(
                            CpuRenderer::new(target)
                                .unwrap()
                                .render(&expected.iter().map(|p| p.rgb).collect::<Vec<_>>())
                                .unwrap(),
                        ),
                        _ => None,
                    };
                    let mut maximum = 0_f64;
                    let mut max_codes = 0_u16;
                    for (i, p) in expected.iter().enumerate() {
                        let point = [
                            origin[0] + i as u32 % TILE[0],
                            origin[1] + i as u32 / TILE[0],
                        ];
                        // Padded originals are never consumed by reconstruction and bypass local stages.
                        if point[0] >= area.x + area.width || point[1] >= area.y + area.height {
                            continue;
                        }
                        for c in 0..3 {
                            let expected = rendered.as_ref().map_or(p.rgb[c], |v| v[i][c] as f64);
                            let error = (actual[i][c] - expected).abs();
                            maximum = maximum.max(error);
                            let allowed = if rendered.is_some() {
                                0.0015
                            } else {
                                2e-12 + 2e-11 * expected.abs()
                            };
                            assert!(
                                error <= allowed,
                                "corrected={corrected} format={format:?} point={point:?} c={c}: {} vs {expected}",
                                actual[i][c]
                            );
                        }
                        if let Format::Exr(_) = format {
                            let flags = p
                                .floored
                                .iter()
                                .enumerate()
                                .fold(0, |v, (c, f)| v | (u32::from(*f) << c));
                            assert_eq!(actual[i][3] as u32, flags);
                        }
                        if let Format::Tiff(target) = format {
                            let a = target
                                .encode_u16(&[glam::Vec3::from_array([
                                    actual[i][0] as f32,
                                    actual[i][1] as f32,
                                    actual[i][2] as f32,
                                ])])
                                .unwrap()[0];
                            let b = target.encode_u16(&[rendered.as_ref().unwrap()[i]]).unwrap()[0];
                            max_codes = max_codes.max(
                                a.into_iter()
                                    .zip(b)
                                    .map(|(a, b)| a.abs_diff(b))
                                    .max()
                                    .unwrap(),
                            );
                        }
                    }
                    if rendered.is_some() {
                        assert!(
                            max_codes <= 1,
                            "original TIFF sample diverged beyond one RGB16 code"
                        );
                    }
                    println!(
                        "export parity corrected={corrected} locals={} format={format:?} origin={origin:?} max_absolute={maximum} max_RGB16_codes={max_codes}",
                        snapshot.locals().len()
                    );
                }
            }
        }
    }
}

#[test]
fn block_reconstruction_keeps_exact_coverage_and_native_permutations_across_tile_edges() {
    let area = SourceRegion::from([10, 20, 513, 65]);
    let rgb = |point: [u32; 2]| {
        let c = codes(point);
        DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64)
    };
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        for angle in [0., 0.13, -0.2] {
            let composition = Composition::with_crop(
                area,
                RollOrientation {
                    turns,
                    horizontal: true,
                    ..Default::default()
                },
                PhotoOrientation {
                    flips: Flips {
                        horizontal: false,
                        vertical: true,
                    },
                    ..Default::default()
                },
                Some(Crop {
                    center: [266., 52.],
                    size: [128, 16],
                    angle,
                }),
            )
            .unwrap();
            let raster = FrameRaster::full(composition).unwrap();
            let [width, height] = composition.size();
            let actual = reconstruct(composition, [0, 0], [width, height], |origin| {
                Ok(tile(origin, tile_size(area))
                    .into_iter()
                    .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64, 0.])
                    .collect())
            })
            .unwrap();
            for y in 0..height {
                for x in 0..width {
                    let expected = if composition.is_exact() {
                        rgb(composition.source_pixel([x, y]).unwrap())
                    } else {
                        crate::sampling::weighted_mean_iter(
                            raster
                                .footprint([x, y])
                                .unwrap()
                                .pixels(area)
                                .unwrap()
                                .map(|(point, weight)| Ok((rgb(point), weight))),
                        )
                        .unwrap()
                    };
                    assert_eq!(actual[(y * width + x) as usize], expected);
                }
            }
        }
    }
}
