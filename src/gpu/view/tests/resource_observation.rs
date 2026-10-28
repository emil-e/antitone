//! Bounded authored workload; run alone for comparable cold/warm and allocator observations.
use super::*;
use crate::{adjustment_layers::AdjustmentLayer, gpu::ProcessingInput};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

#[test]
#[ignore = "isolated authored 1536-square source, cold/warm execution and sampled resource observation"]
fn acquisition_execution_resources() {
    crate::cache_observation::with_monitor(|monitor, device, queue| {
        let file = crate::dng_fixture::Fixture {
            side: 1536,
            bits: 16,
            ramp: true,
            ..Default::default()
        }
        .write();
        let scan = Scan::open(file.path(), None).map_err(|e| e.to_string())?;
        let area = scan.area;
        let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::new(
            composition,
            FramePoint([256., 256.]),
            FramePoint([1216., 976.]),
            [640, 480],
        )
        .unwrap();
        let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
        let settings = crate::film_emulation::FilmEmulationPreset::Medium.settings();
        let gate = SubmissionGate::default();
        let peak = Arc::new(AtomicU64::new(0));
        let (observed, observed_device) = (peak.clone(), device.clone());
        gate.observe_encoded_resources(Arc::new(move || {
            observed.fetch_max(
                crate::gpu::allocated_resource_bytes(&observed_device).unwrap() as u64,
                Ordering::Relaxed,
            );
        }));
        let checkpoint = |name: &str, started: Instant| {
            let [creations, cells] = crate::gpu::test_support::take_original_image_creations();
            println!(
                concat!(
                    "execution_resource phase={} seconds={:.9} retained_device={} ",
                    "encoded_device_peak={} original_image_creations={} original_image_cells={}"
                ),
                name,
                started.elapsed().as_secs_f64(),
                crate::gpu::allocated_resource_bytes(device).unwrap(),
                peak.load(Ordering::Relaxed),
                creations,
                cells,
            );
        };
        let start = Instant::now();
        let renderer =
            pollster::block_on(crate::renderer::GpuRenderer::new(device, queue)).unwrap();
        let kernels = pollster::block_on(ViewKernels::new(device)).unwrap();
        checkpoint("setup", start);

        let make_input = |ev: f64| {
            ProcessingInput::from(Arc::new(
                GpuPipeline::new(
                    device,
                    InputSettings::Digital,
                    Default::default(),
                    AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(
                        AdjustmentLayerSettings {
                            exposure: ev,
                            ..Default::default()
                        },
                    )])
                    .unwrap(),
                    matrix,
                    area,
                )
                .unwrap(),
            ))
            .with_film_emulation(settings)
            .with_frame_size(composition.crop().size)
            .with_film_seed(17)
        };
        let input = make_input(0.);
        let spec = ViewSourceSpec::new(1, frame, InputSettings::Digital, matrix, None)
            .unwrap()
            .with_pipeline(input.pixel_pipeline().unwrap())
            .unwrap();
        let start = Instant::now();
        peak.store(0, Ordering::Relaxed);
        let mut sources = ViewSource::acquire_many(
            device,
            queue,
            &gate,
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            vec![spec.clone(), spec.underlay().unwrap()],
            || monitor.check().is_err(),
        )
        .unwrap()
        .unwrap();
        checkpoint("cold_acquisition", start);
        let underlay = sources.pop().unwrap();
        let detail = sources.pop().unwrap();
        let mut previous = None;
        let mut previous_underlay = None;
        let context = egui::Context::default();
        for edit in 0..7 {
            monitor.check()?;
            let input = make_input(edit as f64 * 0.1);
            let start = Instant::now();
            peak.store(0, Ordering::Relaxed);
            let view = RenderedView::new(
                device,
                detail.clone(),
                &input,
                OutputTarget::Srgb,
                previous.as_ref(),
            )
            .unwrap();
            let background = RenderedView::new(
                device,
                underlay.clone(),
                &input,
                OutputTarget::Srgb,
                previous_underlay.as_ref(),
            )
            .unwrap();
            view.ensure_submitted(device, queue, &gate, &context);
            background.ensure_submitted(device, queue, &gate, &context);
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            checkpoint(
                if edit == 0 {
                    "cold_grade"
                } else {
                    "warm_grade"
                },
                start,
            );
            previous = Some(view);
            previous_underlay = Some(background);
        }

        let input = make_input(0.);
        for run in 0..4 {
            monitor.check()?;
            let start = Instant::now();
            peak.store(0, Ordering::Relaxed);
            let request = crate::refinement::ViewRequest {
                area: frame.bounds().unwrap(),
                output: [67, 53],
                frame: Some(FrameRaster {
                    output: [67, 53],
                    ..frame
                }),
            };
            let image = crate::refinement::refine(
                &renderer,
                device,
                (queue, &gate),
                (&scan, None),
                input.clone(),
                OutputTarget::Srgb,
                (&AtomicU64::new(1), 1),
                request,
            )
            .unwrap()
            .unwrap();
            checkpoint(
                if run == 0 {
                    "cold_regional"
                } else {
                    "warm_regional"
                },
                start,
            );
            if let Some(prefix) = std::env::var_os("ANTITONE_RESOURCE_OUTPUT") {
                let pixels = crate::viewer::readback::image_pixels(device, queue, &image);
                std::fs::write(
                    std::path::PathBuf::from(prefix).with_extension("rgbaf32"),
                    bytemuck::cast_slice(&pixels),
                )
                .unwrap();
            }
        }
        if let Some(prefix) = std::env::var_os("ANTITONE_RESOURCE_OUTPUT") {
            let pixels = crate::viewer::readback::image_pixels(
                device,
                queue,
                &previous.as_ref().unwrap().image,
            );
            std::fs::write(
                std::path::PathBuf::from(prefix).with_extension("view.rgbaf32"),
                bytemuck::cast_slice(&pixels),
            )
            .unwrap();
        }
        drop(previous);
        drop(previous_underlay);
        drop(detail);
        drop(underlay);
        crate::cache_observation::wait(device, queue).unwrap();
        checkpoint("retired_view", Instant::now());
        Ok(())
    })
    .unwrap();
}
