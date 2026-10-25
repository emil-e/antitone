use super::*;
use crate::raw_test_support::train_raw;

#[test]
#[ignore = "read-only diagnostic RAW exact-response/prefix reuse probe; not photographic or device acceptance"]
fn real_raw_response_and_prefix_reuse() {
    let (scan, calibration) = train_raw::load();
    let (width, height, pixels) = scan.preview_source(1024).unwrap();
    let a = scan.area;
    let geometry = SourceGeometry::new([a.x, a.y, a.width, a.height], [width, height]).unwrap();
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        println!(
            "reuse_raw camera={} size=[{width},{height}] adapter={:?} samples=9 timing=encode_submit_gpu_completion excludes=raw_development_upload_readback_rendering_reconstruction",
            scan.camera_model,
            adapter.get_info()
        );
        let baseline = ProcessingKernel::new(&device).await.unwrap();
        let probe = Probe::new(&device, pixels.len()).await;
        let mut source =
            ProcessingSource::new_continuous(&device, &pixels, scan.camera_to_ap1).unwrap();
        source.set_geometry(geometry).unwrap();
        for count in [0, 2] {
            let snapshot = snapshot(PhotoSettings::default(), count, true);
            let grade = GpuPhotoGrade::new(
                &device,
                calibration,
                RollSettings::default(),
                snapshot.clone(),
                scan.camera_to_ap1,
                a,
            )
            .unwrap();
            let run = |steps: Option<&[usize]>| {
                let start = Instant::now();
                let mut encoder = device.create_command_encoder(&Default::default());
                if let Some(steps) = steps {
                    probe.encode(&device, &mut encoder, &source, &grade, steps);
                } else {
                    baseline
                        .encode_grade(&device, &mut encoder, &source, &probe.output, &grade)
                        .unwrap();
                }
                let submission = queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .unwrap();
                start.elapsed().as_secs_f64() * 1000.
            };
            run(None);
            let expected = read_words(&device, &queue, &probe.output);
            let scalar = SpatialPipeline::new(
                calibration,
                RollSettings::default(),
                snapshot,
                scan.camera_to_ap1,
            )
            .unwrap();
            let mut signed = 0;
            let mut hdr = 0;
            for (i, pixel) in pixels.iter().enumerate() {
                let reference = scalar
                    .apply(
                        DVec3::from_array(std::array::from_fn(|c| f64::from(pixel[c]) / 65535.)),
                        geometry.pixel(i as u32).unwrap(),
                    )
                    .unwrap();
                signed += usize::from(reference.rgb.min_element() < 0.);
                hdr += usize::from(reference.rgb.max_element() > 1.);
                for c in 0..3 {
                    let actual = f64::from(f32::from_bits(expected[4 * i + c]));
                    assert!(
                        (actual - reference.rgb[c]).abs() <= 2e-5 + 2e-4 * reference.rgb[c].abs(),
                        "RAW count={count} pixel={i} channel={c}"
                    );
                }
                let flags = (0..3).fold(0, |a, c| a | (u32::from(reference.floored[c]) << c));
                assert_eq!(f32::from_bits(expected[4 * i + 3]), flags as f32);
            }
            for (name, build, warm) in [
                ("lookup", 0, 1),
                ("camera-prefix", 2, 3),
                ("global-prefix", 4, 5),
            ] {
                run(Some(&[build, warm]));
                exact(
                    "RAW",
                    name,
                    &expected,
                    &read_words(&device, &queue, &probe.output),
                );
                run(Some(&[warm]));
                exact(
                    "RAW-warm",
                    name,
                    &expected,
                    &read_words(&device, &queue, &probe.output),
                );
                let mut times = Vec::new();
                let mut cold_times = Vec::new();
                let mut ratios = Vec::new();
                let mut anchors = Vec::new();
                for sample in 0..9 {
                    let before = run(None);
                    let (cold, time) = if sample % 2 == 0 {
                        (run(Some(&[build, warm])), run(Some(&[warm])))
                    } else {
                        let time = run(Some(&[warm]));
                        (run(Some(&[build, warm])), time)
                    };
                    let after = run(None);
                    times.push(time);
                    cold_times.push(cold);
                    ratios.push(time / ((before + after) / 2.));
                    anchors.extend([before, after]);
                }
                println!(
                    "reuse_raw count={count} candidate={name} baseline_ms={:.3} warm_ms={:.3} warm_ratio={:.4} cold_ms={:.3} exact_pixels={} signed_pixels={signed} hdr_pixels={hdr} response_bytes={} prefix_bytes={}",
                    median(&anchors),
                    median(&times),
                    median(&ratios),
                    median(&cold_times),
                    pixels.len(),
                    probe.response.size(),
                    probe.prefix.size()
                );
            }
        }
    });
}
