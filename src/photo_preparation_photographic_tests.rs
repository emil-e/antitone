//! Opt-in read-only photographs: shared preview latency and camera/zero/initialized panels.
use super::*;
use crate::rendering::{CpuRenderer, OutputTarget};
use std::time::Instant;

#[test]
#[ignore = "read-only private RAWs from ANTITONE_EXPOSURE_SOURCES; optional panels in ANTITONE_EXPOSURE_OUTPUT"]
fn photographic_reference_initialization() {
    let paths =
        std::env::var_os("ANTITONE_EXPOSURE_SOURCES").expect("set a platform-separated path list");
    let output = std::env::var_os("ANTITONE_EXPOSURE_OUTPUT").map(std::path::PathBuf::from);
    if let Some(output) = &output {
        std::fs::create_dir_all(output).unwrap();
    }
    for path in std::env::split_paths(&paths) {
        let start = Instant::now();
        let raw = crate::raw::RawFile::open(&path).unwrap();
        let wb = raw.fixed_wb(raw.metadata.as_shot_wb());
        let file = raw.file.clone();
        let development = SourceDevelopment::from_metadata(&raw.metadata, wb).unwrap();
        let header_ms = start.elapsed().as_secs_f64() * 1000.;
        let store = SourceStore::new(128 << 20);
        let tick = Instant::now();
        let preview = store
            .preview(&file, || embedded_preview::read(&path, &development))
            .unwrap();
        let preview_ms = tick.elapsed().as_secs_f64() * 1000.;
        let tick = Instant::now();
        let scan = Scan::from_raw(raw, Some(wb)).unwrap();
        let develop_ms = tick.elapsed().as_secs_f64() * 1000.;
        let tick = Instant::now();
        let prepared = store
            .prepare_source(InputSettings::Digital, scan.source())
            .unwrap();
        let proxy = prepared.proxy;
        let prepare_ms = tick.elapsed().as_secs_f64() * 1000.;
        let reference = scan.source().reference_exposure.get().unwrap();
        let tick = Instant::now();
        for _ in 0..10 {
            assert!(
                store
                    .prepare_source(InputSettings::Digital, scan.source())
                    .unwrap()
                    .proxy
                    .is_none()
            );
        }
        let cached_us = tick.elapsed().as_secs_f64() * 100_000.;
        println!(
            "REFERENCE {} header_ms={header_ms:.3} preview_ms={preview_ms:.3} develop_ms={develop_ms:.3} prepare_ms={prepare_ms:.3} cached_us={cached_us:.3} preview={:?} outcome={reference:?}",
            path.display(),
            preview.as_ref().map(|p| p.rgb.dimensions())
        );
        let (Some(output), Some(preview), Some(proxy)) = (&output, preview, proxy) else {
            continue;
        };
        let renderer = CpuRenderer::new(OutputTarget::Srgb).unwrap();
        let camera = proxy
            .pixels
            .iter()
            .map(|p| glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])) / 65535.)
            .collect::<Vec<_>>();
        let render = |matrix: glam::DMat3| {
            let pixels = renderer
                .render(&camera.iter().map(|p| matrix * *p).collect::<Vec<_>>())
                .unwrap();
            let data = pixels
                .into_iter()
                .flat_map(|p| {
                    p.to_array().map(|v| {
                        (OutputTarget::Srgb.encode(f64::from(v)).unwrap() * 255.).round() as u8
                    })
                })
                .collect();
            let image = image::RgbImage::from_raw(proxy.width, proxy.height, data).unwrap();
            match preview.capture_flip {
                0 => image,
                3 => image::imageops::rotate180(&image),
                5 => image::imageops::rotate270(&image),
                6 => image::imageops::rotate90(&image),
                _ => image,
            }
        };
        let zero = render(proxy.matrix);
        let initialized = render(proxy.positive_matrix(&InputSettings::Digital).unwrap());
        let Some(frame) = reference_frame(
            &preview.rgb,
            f64::from(zero.width()) / f64::from(zero.height()),
        ) else {
            preview
                .rgb
                .save(output.join(format!(
                    "{}-unmatched-camera.png",
                    path.file_stem().unwrap().to_string_lossy()
                )))
                .unwrap();
            continue;
        };
        let camera =
            image::imageops::crop_imm(&preview.rgb, frame[0], frame[1], frame[2], frame[3])
                .to_image();
        let camera = image::imageops::resize(
            &camera,
            zero.width(),
            zero.height(),
            image::imageops::FilterType::Triangle,
        );
        let mut panels = image::RgbImage::new(zero.width() * 3, zero.height());
        for (i, image) in [camera, zero, initialized].into_iter().enumerate() {
            image::imageops::replace(&mut panels, &image, i as i64 * i64::from(image.width()), 0);
        }
        panels
            .save(output.join(format!(
                "{}.png",
                path.file_stem().unwrap().to_string_lossy()
            )))
            .unwrap();
    }
}
