//! Isolated authored native-output cost comparison, including codec work.
use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[test]
#[ignore = "isolated authored native output latency and allocator diagnostic"]
fn native_output_resources() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 1536,
        bits: 16,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = crate::scan::Scan::open(file.path(), None).unwrap();
    let composition = crate::geometry::Composition::with_crop(
        scan.area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(scan.area.x) + f64::from(scan.area.width) / 2.,
                f64::from(scan.area.y) + f64::from(scan.area.height) / 2.,
            ],
            size: [scan.area.width * 3 / 4, scan.area.height * 3 / 4],
            angle: 0.17,
        }),
    )
    .unwrap();
    let format = match std::env::var("ANTITONE_NATIVE_FORMAT").as_deref() {
        Ok("half") => Format::Exr(crate::export::ExrPrecision::Half),
        Ok("full") => Format::Exr(crate::export::ExrPrecision::Full),
        Ok("tiff") | Err(std::env::VarError::NotPresent) => {
            Format::Tiff(crate::rendering::OutputTarget::Srgb)
        }
        _ => panic!("unsupported ANTITONE_NATIVE_FORMAT; use tiff, half or full"),
    };
    let grade = crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        crate::adjustment_layers::AdjustmentLayer::global(
            crate::adjustment_layers::AdjustmentLayerSettings {
                exposure: -0.7,
                contrast: 1.1,
                ..Default::default()
            },
        ),
    ])
    .unwrap();
    let film = crate::film_emulation::FilmEmulationSettings {
        enabled: true,
        size: 0.1,
        amount: 0.055,
        colour: 0.7,
    };
    let directory =
        tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache"))
            .unwrap();
    let path = directory.path().join("native-output");
    let peak = Arc::new(AtomicU64::new(0));
    let gate = crate::gpu::SubmissionGate::default();
    let observed = peak.clone();
    let gpu = device.clone();
    gate.observe_encoded_resources(Arc::new(move || {
        observed.fetch_max(
            crate::gpu::allocated_resource_bytes(&gpu).unwrap_or(0) as u64,
            Ordering::Relaxed,
        );
    }));
    for run in 0..6 {
        peak.store(0, Ordering::Relaxed);
        let started = Instant::now();
        let write = || {
            if std::env::var_os("ANTITONE_NATIVE_GPU_ONLY").is_some() {
                use crate::gpu::execution;
                let (output, encoding) = match format {
                    Format::Tiff(target) => (
                        crate::rendering::OutputReference::DisplayReferred(target),
                        execution::SampleEncoding::Rgb16,
                    ),
                    _ => (
                        crate::rendering::OutputReference::SceneReferred,
                        execution::SampleEncoding::LinearFloat,
                    ),
                };
                return execution::execute(
                    &renderer,
                    &device,
                    (&queue, &gate),
                    execution::Request {
                        scan: &scan,
                        field: None,
                        input: crate::processing::InputSettings::Digital,
                        roll: Default::default(),
                        grade: grade.clone(),
                        repairs: &[],
                        composition,
                        stages: crate::pixel_pipeline::PixelPipeline::film(
                            film,
                            composition.size(),
                            (1 << 60) + 17,
                        )
                        .unwrap(),
                        output,
                        look: None,
                        encoding,
                        budget: execution::WorkBudget::for_device(&device),
                    },
                    |render, complete| {
                        let size = composition.size();
                        for y in (0..size[1]).step_by(128) {
                            for x in (0..size[0]).step_by(128) {
                                let output =
                                    render([x, y], [128.min(size[0] - x), 128.min(size[1] - y)])?;
                                std::hint::black_box(output);
                            }
                        }
                        complete()
                    },
                    || Ok(()),
                );
            }
            crate::gpu_export::area::write(
                &renderer,
                &device,
                (&queue, &gate),
                Request {
                    scan: &scan,
                    field: None,
                    input: crate::processing::InputSettings::Digital,
                    roll: Default::default(),
                    grade: grade.clone(),
                    repairs: &[],
                    composition,
                    format,
                    film_emulation: film,
                    film_seed: (1 << 60) + 17,
                    look: None,
                },
                (&path, crate::export::Overwrite::Confirmed),
                || Ok(()),
            )
        };
        let (stats, timings) = if std::env::var_os("ANTITONE_NATIVE_PHASES").is_some() {
            let (result, timings) = crate::export_timing::record(write);
            (result.unwrap(), Some(timings))
        } else {
            (write().unwrap(), None)
        };
        let seconds = started.elapsed().as_secs_f64();
        if let Some(timings) = timings {
            timings.report(started.elapsed());
        }
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
            0
        );
        let usage = unsafe { usage.assume_init() };
        println!(
            "native_output run={run} seconds={seconds:.9} retained={} encoded_peak={} source_cells={} submissions={} peak_rss={} file_bytes={}",
            crate::gpu::allocated_resource_bytes(&device).unwrap_or(0),
            peak.load(Ordering::Relaxed),
            stats.source_pixels,
            stats.submissions,
            usage.ru_maxrss,
            std::fs::metadata(&path).map_or(0, |m| m.len())
        );
    }
    if let Some(other) = std::env::var_os("ANTITONE_NATIVE_COMPARE") {
        let values = |path: &std::path::Path| -> Vec<f64> {
            match format {
                Format::Tiff(_) => {
                    let mut decoder =
                        tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
                    let tiff::decoder::DecodingResult::U16(data) = decoder.read_image().unwrap()
                    else {
                        panic!("RGB16 capture")
                    };
                    data.into_iter().map(f64::from).collect()
                }
                Format::Exr(_) => {
                    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
                    image
                        .layer_data
                        .channel_data
                        .list
                        .into_iter()
                        .flat_map(|channel| match channel.sample_data {
                            exr::prelude::FlatSamples::F16(data) => {
                                data.into_iter().map(|v| v.to_f64()).collect::<Vec<_>>()
                            }
                            exr::prelude::FlatSamples::F32(data) => {
                                data.into_iter().map(f64::from).collect()
                            }
                            _ => panic!("float capture"),
                        })
                        .collect()
                }
            }
        };
        let a = values(&path);
        let b = values(std::path::Path::new(&other));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().chain(&b).all(|v| v.is_finite()));
        let difference = a
            .iter()
            .zip(&b)
            .map(|(a, b)| (a - b).abs())
            .fold(0., f64::max);
        println!(
            "native_output comparison_components={} max_abs_error={difference:.12}",
            a.len()
        );
    }
    if let Some(output) = std::env::var_os("ANTITONE_NATIVE_CAPTURE") {
        std::fs::copy(path, output).unwrap();
    }
}
