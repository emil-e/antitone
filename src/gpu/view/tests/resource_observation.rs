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
            let [creations, bytes] = crate::gpu::test_support::take_evaluation_buffer_creations();
            println!(
                concat!(
                    "execution_resource phase={} seconds={:.9} retained_device={} ",
                    "encoded_device_peak={} evaluation_buffer_creations={} evaluation_buffer_bytes={}"
                ),
                name,
                started.elapsed().as_secs_f64(),
                crate::gpu::allocated_resource_bytes(device).unwrap(),
                peak.load(Ordering::Relaxed),
                creations,
                bytes,
            );
        };
        let start = Instant::now();
        let mut policy = crate::gpu::evaluation::CachePolicy::default();
        if let Some(value) = std::env::var_os("ANTITONE_GENERATED_CACHE_BYTES") {
            policy.generated_bytes = value
                .to_str()
                .ok_or("cache budget is not UTF-8")?
                .parse::<u64>()
                .map_err(|e| e.to_string())?;
        }
        let renderer = pollster::block_on(crate::renderer::GpuRenderer::new_with_cache_policy(
            device,
            queue,
            SubmissionGate::default(),
            policy,
        ))
        .unwrap();
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

#[test]
#[ignore = "isolated neutral-image retention/regeneration and actual allocator observation"]
fn neutral_image_retention_resources() {
    crate::cache_observation::with_monitor(|monitor,device,queue|{
        let file=crate::dng_fixture::Fixture{side:1536,bits:16,ramp:true,..Default::default()}.write();
        let scan=Scan::open(file.path(),None).map_err(|e|e.to_string())?;
        let composition=Composition::new(scan.area,Default::default(),Default::default()).unwrap();
        let frame=FrameRaster::new(composition,FramePoint([256.,256.]),FramePoint([1216.,976.]),[640,480]).unwrap();
        let policy=crate::gpu::evaluation::CachePolicy{generated_bytes:std::env::var("ANTITONE_GENERATED_CACHE_BYTES").ok().map(|v|v.parse::<u64>().unwrap()).unwrap_or(crate::gpu::evaluation::CachePolicy::default().generated_bytes)};
        let renderer=pollster::block_on(crate::renderer::GpuRenderer::new_with_cache_policy(device,queue,Default::default(),policy)).unwrap();
        let kernels=pollster::block_on(ViewKernels::new(device)).unwrap();
        let cache=renderer.evaluation().image_cache(2);
        let spec=ViewSourceSpec::new(1,frame,InputSettings::Digital,scan.positive_matrix(&InputSettings::Digital).unwrap(),None).unwrap().with_identity(crate::gpu::execution::source::Identity::camera(scan.source().identity()));
        let gate=SubmissionGate::default();
        let peak=Arc::new(AtomicU64::new(0));let observed=peak.clone();let observed_device=device.clone();
        gate.observe_encoded_resources(Arc::new(move||{observed.fetch_max(crate::gpu::allocated_resource_bytes(&observed_device).unwrap() as u64,Ordering::Relaxed);}));
        let mut reference=None;
        for run in 0..4 {
            monitor.check()?;
            peak.store(0,Ordering::Relaxed);
            let _=crate::gpu::test_support::take_evaluation_buffer_creations();
            let start=Instant::now();
            let cached=ViewSource::cached_neutral(&cache,renderer.clone(),kernels.clone(),&spec);
            let hit=cached.is_some();
            let source=if let Some(source)=cached{source}else{ViewSource::acquire(device,queue,&gate,renderer.clone(),kernels.clone(),&scan,None,spec.clone(),||monitor.check().is_err()).unwrap().unwrap()};
            source.retain_neutral(&cache);
            let [creations,bytes]=crate::gpu::test_support::take_evaluation_buffer_creations();
            println!("neutral_retention run={run} hit={hit} seconds={:.9} retained_payload={} retained_device={} encoded_device_peak={} evaluation_buffer_creations={creations} evaluation_buffer_bytes={bytes}",start.elapsed().as_secs_f64(),renderer.evaluation().retained_bytes(),crate::gpu::allocated_resource_bytes(device).unwrap(),peak.load(Ordering::Relaxed));
            let pixels=read(device,queue,&source.pixels);
            if let Some(expected)=&reference{assert_eq!(&pixels,expected);}else{reference=Some(pixels);}
            drop(source);
        }
        if let Some(prefix)=std::env::var_os("ANTITONE_RESOURCE_OUTPUT") {std::fs::write(std::path::PathBuf::from(prefix).with_extension("neutral.rgbaf32"),bytemuck::cast_slice(reference.as_ref().unwrap())).unwrap();}
        cache.clear();crate::cache_observation::wait(device,queue).unwrap();
        assert_eq!(renderer.evaluation().retained_bytes(),0);
        println!("neutral_retention retired_device={}",crate::gpu::allocated_resource_bytes(device).unwrap());
        Ok(())
    }).unwrap();
}
