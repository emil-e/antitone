use super::*;
use crate::film_emulation::{FilmEmulationPreset, FilmEmulationSettings};

fn processed(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernels: &crate::gpu::pixel_pipeline::Kernels,
    bounds: SourceRegion,
    settings: FilmEmulationSettings,
) -> Vec<[f32; 4]> {
    processed_with_flags(device, queue, kernels, bounds, settings, 0.)
}

fn processed_with_flags(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernels: &crate::gpu::pixel_pipeline::Kernels,
    bounds: SourceRegion,
    settings: FilmEmulationSettings,
    flags: f32,
) -> Vec<[f32; 4]> {
    let values: Vec<_> = (bounds.y..bounds.y + bounds.height)
        .flat_map(|y| {
            (bounds.x..bounds.x + bounds.width).map(move |x| {
                [
                    if x >= 1080 { 0.4 } else { -0.03 },
                    0.12 + (y % 17) as f32 * 0.002,
                    1.8 + (x % 23) as f32 * 0.01,
                    flags,
                ]
            })
        })
        .collect();
    let working = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&values),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: working.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let plan = crate::pixel_pipeline::PixelPipeline::film(settings, 6000, (1 << 60) + 17).unwrap();
    let prepared = crate::gpu::pixel_pipeline::Prepared::new(
        device,
        &plan,
        crate::gpu::raster::Raster::native(bounds),
        None,
    )
    .unwrap();
    kernels
        .encode(device, &mut encoder, &prepared, &working)
        .unwrap();
    encoder.copy_buffer_to_buffer(&working, 0, &staging, 0, working.size());
    let submission = queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    bytemuck::cast_slice(&mapped).to_vec()
}

#[test]
fn native_floor_flags_and_zero_amount_retain_the_independent_source_gaussian_mtf() {
    let (device, queue, _) = super::super::tests::context();
    let kernels = pollster::block_on(crate::gpu::pixel_pipeline::Kernels::new(&device)).unwrap();
    let bounds: SourceRegion = [1048, 980, 64, 64].into();
    let settings = FilmEmulationSettings {
        amount: 0.,
        ..FilmEmulationPreset::Medium.settings()
    };
    // Medium at original height6000 is sigma1.8; this is independent of the GPU stencil.
    let sigma = 1.8_f64;
    let radius = (4. * sigma).ceil() as i32;
    let mass: f64 = (-radius..=radius)
        .map(|i| (-0.5 * (i * i) as f64 / (sigma * sigma)).exp())
        .sum();
    for flags in [0., 7.] {
        let active = processed_with_flags(&device, &queue, &kernels, bounds, settings, flags);
        let clean = processed_with_flags(
            &device,
            &queue,
            &kernels,
            bounds,
            FilmEmulationSettings::default(),
            flags,
        );
        for x in [1076, 1079, 1080, 1083] {
            let expected: f64 = (-radius..=radius)
                .map(|i| {
                    let value = if x + i >= 1080 { 0.4_f32 } else { -0.03_f32 };
                    (-0.5 * (i * i) as f64 / (sigma * sigma)).exp() * value as f64 / mass
                })
                .sum();
            let at = ((1010 - bounds.y) * bounds.width + x as u32 - bounds.x) as usize;
            assert!(
                (active[at][0] as f64 - expected).abs() < 0.000002,
                "native Gaussian x={x}, flags={flags}"
            );
            assert_eq!(clean[at][0], if x >= 1080 { 0.4_f32 } else { -0.03_f32 });
            assert_eq!(active[at][3], flags);
        }
    }
}

#[test]
fn original_film_tiles_match_whole_source_filtering_across_seams_and_rotated_crop_edges() {
    let (device, queue, _) = super::super::tests::context();
    let kernels = pollster::block_on(crate::gpu::pixel_pipeline::Kernels::new(&device)).unwrap();
    let area: SourceRegion = [100, 80, 6000, 6000].into();
    let whole: SourceRegion = [900, 950, 350, 350].into();
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [1080., 1100.],
            size: [240, 220],
            angle: 0.13,
        }),
    )
    .unwrap();
    let settings = FilmEmulationPreset::Coarse.settings();
    let reference = processed(&device, &queue, &kernels, whole, settings);
    let engine = Reconstructor::new(&device).unwrap();
    for origin in [[0, 0], [128, 0], [0, 128], [128, 128]] {
        let extent = std::array::from_fn(|i| (composition.size()[i] - origin[i]).min(128));
        let mut unit = Unit::with_support(
            composition,
            origin,
            extent,
            crate::pixel_pipeline::PixelPipeline::film(settings, area.height, 17)
                .unwrap()
                .support([1., 1.])
                .unwrap(),
        )
        .unwrap();
        unit.mapping.space[0] = 1;
        let b = unit.bounds;
        assert!(
            b.x >= whole.x
                && b.y >= whole.y
                && b.x + b.width <= whole.x + whole.width
                && b.y + b.height <= whole.y + whole.height
        );
        let tile = processed(&device, &queue, &kernels, b, settings);
        let actual = super::tests::reconstructed(
            &device,
            &queue,
            &engine,
            &unit,
            |[x, y]| tile[((y - b.y) * b.width + x - b.x) as usize],
            None,
        );
        for y in 0..extent[1] {
            for x in 0..extent[0] {
                let coverage = composition
                    .coverage(
                        FramePoint([(origin[0] + x) as f64, (origin[1] + y) as f64]),
                        FramePoint([(origin[0] + x + 1) as f64, (origin[1] + y + 1) as f64]),
                    )
                    .unwrap();
                let mut expected = [0.; 3];
                let mut total = 0.;
                for ([sx, sy], weight) in coverage.pixels(area).unwrap() {
                    let p = reference[((sy - whole.y) * whole.width + sx - whole.x) as usize];
                    for c in 0..3 {
                        expected[c] += p[c] as f64 * weight;
                    }
                    total += weight;
                }
                for c in 0..3 {
                    assert!(
                        (actual[(y * extent[0] + x) as usize][c] as f64 - expected[c] / total)
                            .abs()
                            < 0.00008,
                        "tile {origin:?}, pixel ({x},{y}), channel {c}"
                    );
                }
            }
        }
    }
}

#[test]
fn film_halo_coverage_matches_explicit_neighbour_cell_relevance() {
    let area: SourceRegion = [0, 0, 100, 100].into();
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [50., 50.],
            size: [20, 16],
            angle: 0.31,
        }),
    )
    .unwrap();
    let coverage = composition
        .coverage(
            FramePoint([0.; 2]),
            FramePoint(composition.size().map(f64::from)),
        )
        .unwrap();
    for y in 30..70 {
        for x in 30..70 {
            for r in [0u32, 1, 3] {
                let region = SourceRegion::from([x - r, y - r, 2 * r + 1, 2 * r + 1]);
                let explicit = (region.y..region.y + region.height).any(|sy| {
                    (region.x..region.x + region.width).any(|sx| coverage.weight([sx, sy]) > 0.)
                });
                assert_eq!(
                    coverage.intersection_area(region) > 0.,
                    explicit,
                    "({x},{y}), r={r}"
                );
            }
        }
    }
}
