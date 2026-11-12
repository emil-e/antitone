//! Read-only photographic source-development diagnostic; outputs stay in .cache.
use crate::gpu::{evaluation::*, grading::Grading, photograph::PhotographGraph};
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    processing::{InputSettings, RollSettings},
    renderer::GpuRenderer,
    scan::{ImageSource, Scan},
};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &GpuRenderer,
    image: Value,
    crop: CellRegion,
    size: [u32; 2],
    path: &Path,
) {
    let display = Reference::capture(
        device,
        renderer,
        crate::rendering::OutputReference::DisplayReferred(crate::rendering::OutputTarget::Srgb),
        &Default::default(),
    )
    .unwrap()
    .compose(image, renderer)
    .unwrap();
    let domain = Domain {
        bounds: Some(CellRegion::new([0, 0], size)),
        grid: Grid::axis_aligned(
            [crop.x as f64, crop.y as f64],
            [
                f64::from(crop.width) / f64::from(size[0]),
                f64::from(crop.height) / f64::from(size[1]),
            ],
        ),
        meaning: display.domain().meaning,
    };
    let image = reconstruct(display, domain).unwrap();
    let request = renderer
        .evaluation()
        .request(
            image,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [256, 128],
                scratch_bytes: 64 << 20,
            },
        )
        .unwrap();
    let mut rgb = image::RgbImage::new(size[0], size[1]);
    renderer
        .evaluation()
        .read(
            queue,
            &Default::default(),
            &request,
            || false,
            |region, pixels| {
                for (i, pixel) in pixels.iter().enumerate() {
                    assert!(pixel.iter().all(|v| v.is_finite()));
                    let bytes = std::array::from_fn(|c| {
                        (crate::rendering::OutputTarget::Srgb
                            .encode(f64::from(pixel[c]))
                            .unwrap()
                            .clamp(0., 1.)
                            * 255.)
                            .round() as u8
                    });
                    rgb.put_pixel(
                        region.x as u32 + i as u32 % region.width,
                        region.y as u32 + i as u32 / region.width,
                        image::Rgb(bytes),
                    );
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    rgb.save(path).unwrap();
}

#[test]
#[ignore = "private Bayer/X-Trans/DNG development and original-sample GPU rendering; set ANTITONE_HIGHLIGHT_SAMPLES and enable image/png"]
fn supplied_early_highlights_source_observation() {
    let root = PathBuf::from(
        std::env::var_os("ANTITONE_HIGHLIGHT_SAMPLES").expect("set ANTITONE_HIGHLIGHT_SAMPLES"),
    );
    let output = Path::new(".cache/raw-highlights/photographs");
    std::fs::create_dir_all(output).unwrap();
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    renderer
        .evaluation()
        .materializations()
        .set_retention_enabled(false);
    for (name, relative) in [
        (
            "zv1",
            "non-scans/Sony - ZV-1 - 12bit 12bit compressed (3_2).ARW",
        ),
        ("m9", "dng/leica-m9.dng"),
        ("q2", "dng/leica-q2.dng"),
        ("xtrans", "negative-validation-online/_DSF3321.RAF"),
        ("xtrans-daylight", "negative-validation-online/DSCF4243.RAF"),
        ("gr3", "dng/ricoh-gr-iii.dng"),
        ("gr3x", "dng/ricoh-gr-iiix.dng"),
    ] {
        let mut timings = Vec::new();
        let mut retained = None;
        for _ in 0..3 {
            let start = Instant::now();
            let scan = Scan::open(&root.join(relative), None).unwrap();
            timings.push(start.elapsed().as_secs_f64() * 1000.);
            retained = Some(scan);
        }
        let scan = retained.unwrap();
        if name == "zv1" {
            assert!(
                scan.source().metadata().lens_identity().unwrap().is_some(),
                "Sony optics must remain enabled"
            );
        }
        let image = scan.source().camera_image().unwrap();
        let mut hash = blake3::Hasher::new();
        let mut row = vec![[0.; 4]; image.width as usize];
        for y in 0..image.height {
            image
                .copy_samples(
                    crate::raw::Rect::from([0, y, image.width, 1]),
                    &mut row,
                    image.width as usize,
                )
                .unwrap();
            hash.update(bytemuck::cast_slice(&row));
        }
        println!(
            "{name} development_ms={timings:?} continuous={} source_bytes={:?} camera_hash={}",
            image.is_continuous(),
            scan.source().retained_bytes(),
            hash.finalize()
        );
        let bounds: CellRegion = scan.area.try_into().unwrap();
        for ev in [0., -2.] {
            let program = ImageSource::new(&scan, InputSettings::Digital)
                .unwrap()
                .gpu_program()
                .unwrap();
            let grading = Grading::new(
                &device.limits(),
                RollSettings {
                    exposure: ev,
                    ..Default::default()
                },
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                scan.area,
            )
            .unwrap();
            let image = PhotographGraph::original_image(
                &device.limits(),
                crate::gpu::evaluation::camera_source(&scan).unwrap(),
                None,
                program,
                grading,
                &[],
                &Default::default(),
            )
            .unwrap();
            let height =
                (f64::from(bounds.height) * 1280. / f64::from(bounds.width)).round() as u32;
            render(
                &device,
                &queue,
                &renderer,
                image.clone(),
                bounds,
                [1280, height],
                &output.join(format!("{name}-ev{ev}.png")),
            );
            if name == "zv1" {
                render(
                    &device,
                    &queue,
                    &renderer,
                    image.clone(),
                    bounds,
                    [bounds.width, bounds.height],
                    &output.join(format!("{name}-full-ev{ev}.png")),
                );
            }
            let crop = match name {
                "m9" => CellRegion::new(
                    [
                        bounds.x + bounds.width as i32 * 3 / 4,
                        bounds.y + bounds.height as i32 / 10,
                    ],
                    [768, 768],
                ),
                "q2" => CellRegion::new([3304, 2952], [768, 768]),
                "xtrans" => CellRegion::new([1300, 0], [2048, 1200]),
                _ => CellRegion::new([bounds.x + bounds.width as i32 / 3, bounds.y], [768, 384]),
            };
            render(
                &device,
                &queue,
                &renderer,
                image,
                crop,
                [crop.width, crop.height],
                &output.join(format!("{name}-detail-ev{ev}.png")),
            );
        }
    }
}
