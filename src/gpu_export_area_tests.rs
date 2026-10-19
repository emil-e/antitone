use super::*;
use crate::{
    crop::Crop,
    geometry::{PhotoOrientation, QuarterTurns, RollOrientation},
};

fn composition(area: SourceRegion, angle: f64) -> Composition {
    Composition::with_crop(
        area,
        RollOrientation {
            turns: QuarterTurns::One,
            ..Default::default()
        },
        PhotoOrientation::default(),
        Some(Crop {
            center: [
                f64::from(area.x) + f64::from(area.width) / 2.,
                f64::from(area.y) + f64::from(area.height) / 2.,
            ],
            size: [area.width - 180, area.height - 180],
            angle,
        }),
    )
    .unwrap()
}

pub(super) fn reconstructed(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    engine: &Reconstructor,
    unit: &Unit,
    pixel: impl Fn([u32; 2]) -> [f32; 4],
    invalid_working: Option<[u32; 2]>,
) -> Vec<[f32; 4]> {
    let bounds = unit.bounds;
    let pixels: Vec<_> = (bounds.y..bounds.y + bounds.height)
        .flat_map(|y| (bounds.x..bounds.x + bounds.width).map(move |x| [x, y]))
        .map(&pixel)
        .collect();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: bounds.width,
            height: bounds.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bounds.width * 16),
            rows_per_image: Some(bounds.height),
        },
        texture.size(),
    );
    let mut working = pixels.clone();
    if let Some([x, y]) = invalid_working {
        working[((y - bounds.y) * bounds.width + x - bounds.x) as usize][0] = f32::NAN;
    }
    let working = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&working),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    engine.encode(device, &mut encoder, unit, &texture, &working);
    let submission = SubmissionGate::default().submit(queue, [encoder.finish()]);
    engine
        .read(
            device,
            submission,
            [unit.mapping.sizes[2], unit.mapping.sizes[3]],
        )
        .unwrap()
}

#[test]
fn common_area_retains_small_signal_when_signed_hdr_samples_cancel() {
    let (device, queue, _) = super::super::tests::context();
    let engine = Reconstructor::new(&device).unwrap();
    let composition =
        Composition::new([0, 0, 4, 4].into(), Default::default(), Default::default()).unwrap();
    // Declared half-pixel footprint gives four exactly equal weights; this isolates
    // signed accumulation from mapping/clipping error and export crop restrictions.
    let mut unit = Unit {
        bounds: [0, 0, 3, 3].into(),
        support: None,
        mapping: Mapping {
            sizes: [3, 3, 1, 1],
            origin: [0.5, 0.5, 0., 0.],
            dx: [1., 0., 0., 0.],
            dy: [0., 1., 0., 0.],
            space: [0; 4],
        },
    };
    let pixel = |p: [u32; 2]| {
        let value = match p {
            [0, 0] => 100_000_000.,
            [1, 0] | [1, 1] => 1.,
            [0, 1] => -100_000_000.,
            _ => 0.,
        };
        [value, value, value, 0.]
    };
    let expected = crate::sampling::weighted_mean_iter(
        FrameRaster::new(
            composition,
            FramePoint([0.5, 0.5]),
            FramePoint([1.5, 1.5]),
            [1, 1],
        )
        .unwrap()
        .footprint([0, 0])
        .unwrap()
        .pixels(composition.area())
        .unwrap()
        .map(|(point, weight)| Ok((DVec3::splat(f64::from(pixel(point)[0])), weight))),
    )
    .unwrap();
    assert_eq!(expected, DVec3::splat(0.5));
    for space in [0, 1] {
        unit.mapping.space[0] = space;
        let actual = reconstructed(&device, &queue, &engine, &unit, pixel, None)[0];
        assert_eq!(
            actual,
            [0.5, 0.5, 0.5, 1.],
            "signed HDR cancellation, linear space={space}"
        );
    }
}

#[test]
fn local_area_preserves_constants_ranges_impulses_and_ramps_against_f64_at_origins_and_seams() {
    let (device, queue, _) = super::super::tests::context();
    let engine = Reconstructor::new(&device).unwrap();
    let mut worst = 0_f64;
    for base in [[13, 29], [(1 << 25) + 13, (1 << 25) + 29]] {
        let area: SourceRegion = [base[0], base[1], 611, 557].into();
        for angle in [0.015, -0.271, 0.389] {
            let composition = composition(area, angle);
            let raster = FrameRaster::full(composition).unwrap();
            let [width, height] = composition.size();
            for (origin, extent) in [
                ([0, 0], [128, 128]),
                ([128, 0], [128, 128]),
                ([width - 17, height - 9], [17, 9]),
            ] {
                let unit = Unit::new(composition, origin, extent).unwrap();
                for value in [0_f32, 1_f32] {
                    let actual = reconstructed(
                        &device,
                        &queue,
                        &engine,
                        &unit,
                        |_| [value, value, value, 0.],
                        None,
                    );
                    assert!(
                        actual
                            .iter()
                            .all(|p| p[..3] == [value; 3] && (p[3] - 1.).abs() < 0.0001)
                    );
                }
                let impulse = raster
                    .footprint([origin[0] + extent[0] / 2, origin[1] + extent[1] / 2])
                    .unwrap()
                    .pixels(area)
                    .unwrap()
                    .next()
                    .unwrap()
                    .0;
                let pixel = |p: [u32; 2]| {
                    [
                        if p == impulse { 1. } else { 0. },
                        (p[0] - base[0]) as f32 / 611.,
                        (p[1] - base[1]) as f32 / 557.,
                        0.,
                    ]
                };
                let actual = reconstructed(&device, &queue, &engine, &unit, pixel, None);
                for (i, p) in actual.iter().enumerate() {
                    assert!(
                        p[..3].iter().all(|v| (0. ..=1.).contains(v)),
                        "range must not rely on a clamp: {p:?}"
                    );
                    let point = [
                        origin[0] + i as u32 % extent[0],
                        origin[1] + i as u32 / extent[0],
                    ];
                    let reference = crate::sampling::weighted_mean_iter(
                        raster.footprint(point).unwrap().pixels(area).unwrap().map(
                            |(point, weight)| {
                                let p = pixel(point);
                                Ok((DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64), weight))
                            },
                        ),
                    )
                    .unwrap();
                    for c in 0..3 {
                        worst = worst.max((f64::from(p[c]) - reference[c]).abs());
                    }
                }
            }
        }
    }
    // A diagnostic spatial accuracy bound, not a rendered-photograph acceptance/JND gate.
    println!("export_area authored_max_linear_difference={worst:.9}");
    assert!(worst < 3e-5, "local f32 mapping/weights drifted: {worst}");
}

#[test]
fn native_area_checks_only_contributing_samples_and_rejects_invalid_ap1_or_look_markers() {
    let (device, queue, _) = super::super::tests::context();
    let engine = Reconstructor::new(&device).unwrap();
    let composition = composition([13, 29, 411, 357].into(), 0.015);
    let unit = Unit::new(composition, [128, 128], [3, 3]).unwrap();
    let required = FrameRaster::full(composition)
        .unwrap()
        .footprint([128, 128])
        .unwrap()
        .pixels(composition.area())
        .unwrap()
        .next()
        .unwrap()
        .0;
    let unused = [unit.bounds.x, unit.bounds.y];
    let valid = |_| [0.2, 0.5, 0.9, 0.];
    let actual = reconstructed(&device, &queue, &engine, &unit, valid, Some(required));
    assert!(actual.iter().any(|p| p[3] == -1.));
    let actual = reconstructed(
        &device,
        &queue,
        &engine,
        &unit,
        |p| {
            if p == required {
                [0., 0., 0., -1.]
            } else {
                valid(p)
            }
        },
        None,
    );
    assert!(actual.iter().any(|p| p[3] == -2.));
    let actual = reconstructed(&device, &queue, &engine, &unit, valid, Some(unused));
    assert!(
        actual.iter().all(|p| p[3] > 0.),
        "conservative halo is not photographic demand"
    );
}

#[test]
fn bounded_source_support_covers_fractionally_placed_45_degree_units() {
    let area = SourceRegion::from([13, 29, 800, 800]);
    let mut maximum = 0;
    for step in 0..100 {
        let offset = f64::from(step) / 100.;
        let composition = Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(Crop {
                center: [413. + offset, 429. + offset],
                size: [512, 512],
                angle: std::f64::consts::FRAC_PI_4,
            }),
        )
        .unwrap();
        let unit = Unit::new(composition, [128, 128], [128, 128]).unwrap();
        maximum = maximum.max(unit.bounds.width).max(unit.bounds.height);
    }
    assert_eq!(
        maximum, 185,
        "ceil/floor cell coverage and both halo edges must fit"
    );
}

#[test]
fn integer_source_edges_are_real_cells_without_blank_or_replicated_borders() {
    let (device, queue, _) = super::super::tests::context();
    let engine = Reconstructor::new(&device).unwrap();
    let area: SourceRegion = [19, 37, 131, 133].into();
    let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
    for (origin, extent) in [([0, 0], [128, 128]), ([128, 128], [3, 5])] {
        let unit = Unit::new(composition, origin, extent).unwrap();
        let pixels = reconstructed(
            &device,
            &queue,
            &engine,
            &unit,
            |p| {
                [
                    (p[0] - area.x) as f32 / 131.,
                    (p[1] - area.y) as f32 / 133.,
                    1.,
                    0.,
                ]
            },
            None,
        );
        for (i, p) in pixels.iter().enumerate() {
            assert_eq!(
                *p,
                [
                    (origin[0] + i as u32 % extent[0]) as f32 / 131.,
                    (origin[1] + i as u32 / extent[0]) as f32 / 133.,
                    1.,
                    1.
                ]
            );
        }
    }
    assert!(Unit::new(composition, [130, 0], [2, 1]).is_err());
}
