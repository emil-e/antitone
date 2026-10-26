//! Disposable A/B probes: export one scan, then compare two exported frames.
use super::*;
use std::time::{Duration, Instant};

fn export_scan(output: std::path::PathBuf) {
    let source = std::path::PathBuf::from(std::env::var("AB_SOURCE").expect("AB_SOURCE"));
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(300),
            |editor, cx| editor.start_document(Preparation::New, cx),
        );
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(600),
            |editor, cx| editor.add_raws(vec![source.clone()], cx),
        );
        // Derive the grade the way a user does, so both builds export the same photograph.
        editor = crate::editor::tests::complete_analysis_operation(
            editor,
            Duration::from_secs(1800),
            |editor, cx| editor.begin_analysis(cx),
        );
        editor.sync_document();
        eprintln!("AB analysis status={:?}", editor.status);
        let started = Instant::now();
        editor = crate::editor::tests::complete_export_operation(
            editor,
            Duration::from_secs(1800),
            |editor, cx| {
                editor.export(
                    output.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Confirmed,
                    cx,
                );
            },
        );
        eprintln!(
            "AB export {:?} status={:?}",
            started.elapsed(),
            editor.status
        );
    });
}

#[test]
#[ignore = "private A/B export; set AB_SOURCE and AB_OUTPUT"]
fn ab_export() {
    let output = std::path::PathBuf::from(std::env::var("AB_OUTPUT").expect("AB_OUTPUT"));
    export_scan(output);
}

/// Writes the two exports and an amplified difference as PNGs for the comparison page.
#[test]
#[ignore = "private A/B comparison; run after both exports exist"]
fn ab_compare() {
    let directory = std::path::PathBuf::from(std::env::var("AB_DIR").expect("AB_DIR"));
    let read = |name: &str| -> (u32, u32, Vec<[u16; 3]>) {
        let file = std::fs::File::open(directory.join(name)).unwrap();
        let mut decoder = tiff::decoder::Decoder::new(file)
            .unwrap()
            .with_limits(tiff::decoder::Limits::unlimited());
        let (width, height) = decoder.dimensions().unwrap();
        let pixels = match decoder.read_image().unwrap() {
            tiff::decoder::DecodingResult::U16(values) => values.as_chunks::<3>().0.to_vec(),
            other => panic!("unexpected TIFF sample type: {other:?}"),
        };
        (width, height, pixels)
    };
    let (width, height, before) = read("before.tif");
    let (after_width, after_height, after) = read("after.tif");
    assert_eq!((width, height), (after_width, after_height));
    let mut maximum = 0u16;
    let mut over_one = 0usize;
    let mut over_thirty_two = 0usize;
    let mut diff = Vec::with_capacity(before.len());
    for (a, b) in before.iter().zip(&after) {
        let mut channel = [0u16; 3];
        for c in 0..3 {
            let delta = a[c].abs_diff(b[c]);
            maximum = maximum.max(delta);
            if delta > 1 {
                over_one += 1;
            }
            if delta > 32 {
                over_thirty_two += 1;
            }
            // Amplify x32 so a difference of one code is visible at eight bits.
            channel[c] = (delta as u32 * 32).min(65535) as u16;
        }
        diff.push(channel);
    }
    eprintln!(
        "AB codes: max={maximum} channels_over_1={over_one} channels_over_32={over_thirty_two} of {}",
        before.len() * 3
    );
    let to_image = |pixels: &[[u16; 3]]| {
        let bytes: Vec<u16> = pixels.iter().flatten().copied().collect();
        image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(width, height, bytes).unwrap()
    };

    // A full-frame view at a browsable size, in eight bits.
    let preview = |pixels: &[[u16; 3]]| -> image::RgbImage {
        let wide = to_image(pixels);
        let height = (f64::from(wide.height()) * 1400.0 / f64::from(wide.width())).round() as u32;
        let small =
            image::imageops::resize(&wide, 1400, height, image::imageops::FilterType::Triangle);
        image::DynamicImage::ImageRgb16(small).into_rgb8()
    };
    preview(&before)
        .save(directory.join("frame-before.png"))
        .unwrap();
    preview(&after)
        .save(directory.join("frame-after.png"))
        .unwrap();

    // A 1:1 crop around the densest difference, where any arithmetic change would show.
    let mut best = (0u32, 0u32, 0usize);
    let bin = 64u32;
    let mut counts = std::collections::HashMap::new();
    for (index, channels) in diff.iter().enumerate() {
        if channels.iter().all(|value| *value == 0) {
            continue;
        }
        let (x, y) = ((index as u32) % width, (index as u32) / width);
        *counts.entry((x / bin, y / bin)).or_insert(0usize) += 1;
    }
    for ((bx, by), count) in counts {
        if count > best.2 {
            best = (bx, by, count);
        }
    }
    let crop_w = 1000.min(width);
    let crop_h = 700.min(height);
    let crop_x = (best.0 * bin)
        .saturating_sub(crop_w / 2)
        .min(width - crop_w);
    let crop_y = (best.1 * bin)
        .saturating_sub(crop_h / 2)
        .min(height - crop_h);
    let crop = |pixels: &[[u16; 3]]| -> image::RgbImage {
        let mut out = image::RgbImage::new(crop_w, crop_h);
        for y in 0..crop_h {
            for x in 0..crop_w {
                let source = &pixels[((crop_y + y) * width + crop_x + x) as usize];
                out.put_pixel(
                    x,
                    y,
                    image::Rgb([
                        (source[0] >> 8) as u8,
                        (source[1] >> 8) as u8,
                        (source[2] >> 8) as u8,
                    ]),
                );
            }
        }
        out
    };
    crop(&before)
        .save(directory.join("crop-before.png"))
        .unwrap();
    crop(&after).save(directory.join("crop-after.png")).unwrap();
    // Where the exports differ by at least one code: red, on black.
    let mut hot = image::RgbImage::new(crop_w, crop_h);
    for y in 0..crop_h {
        for x in 0..crop_w {
            let channels = &diff[((crop_y + y) * width + crop_x + x) as usize];
            let value = if channels.iter().any(|channel| *channel > 0) {
                [220, 40, 40]
            } else {
                [0, 0, 0]
            };
            hot.put_pixel(x, y, image::Rgb(value));
        }
    }
    hot.save(directory.join("crop-diff.png")).unwrap();

    // The same overlay for the downscaled frame: a cell is red when any source pixel differs.
    let mut frame_hot =
        image::RgbImage::new(1400, (height as f64 * 1400.0 / width as f64).round() as u32);
    let (hot_w, hot_h) = (frame_hot.width(), frame_hot.height());
    for (index, channels) in diff.iter().enumerate() {
        if channels.iter().all(|value| *value == 0) {
            continue;
        }
        let (x, y) = ((index as u32) % width, (index as u32) / width);
        let (tx, ty) = (x * hot_w / width, y * hot_h / height);
        frame_hot.put_pixel(
            tx.min(hot_w - 1),
            ty.min(hot_h - 1),
            image::Rgb([220, 40, 40]),
        );
    }
    frame_hot.save(directory.join("frame-diff.png")).unwrap();
    eprintln!(
        "AB crop at {crop_x},{crop_y} size {crop_w}x{crop_h}; densest difference cell {best:?}"
    );
}

/// How far apart can the three channels get in a real scan? Density separation per pixel.
#[test]
#[ignore = "private A/B statistic; set AB_DIR"]
fn ab_channel_spread() {
    let directory = std::path::PathBuf::from(std::env::var("AB_DIR").expect("AB_DIR"));
    let file = std::fs::File::open(directory.join("after.tif")).unwrap();
    let mut decoder = tiff::decoder::Decoder::new(file)
        .unwrap()
        .with_limits(tiff::decoder::Limits::unlimited());
    let (width, height) = decoder.dimensions().unwrap();
    let pixels = match decoder.read_image().unwrap() {
        tiff::decoder::DecodingResult::U16(values) => values,
        other => panic!("unexpected TIFF sample type: {other:?}"),
    };
    let mut spreads: Vec<f32> = Vec::with_capacity((width * height) as usize);
    let mut worst = (0f32, [0u16; 3]);
    for pixel in pixels.as_chunks::<3>().0 {
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        for code in pixel {
            // A crude density: darker signal means more dye, which is more absorption.
            let density = -((f32::from(*code).max(1.0)) / 65535.0).log10();
            low = low.min(density);
            high = high.max(density);
        }
        let spread = high - low;
        if spread > worst.0 {
            worst = (spread, [pixel[0], pixel[1], pixel[2]]);
        }
        spreads.push(spread);
    }
    spreads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| spreads[((spreads.len() - 1) as f64 * q) as usize];
    eprintln!(
        "AB channel density spread: median={:.3} p99={:.3} p99.9={:.3} max={:.3} worst_codes={:?} of {} pixels",
        at(0.5),
        at(0.99),
        at(0.999),
        worst.0,
        worst.1,
        spreads.len()
    );
}
