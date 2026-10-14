use super::*;
use crate::renderer::RendererCache;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[path = "editor_import_tests.rs"]
mod import;

#[test]
#[ignore = "read-only local RAW/Metal corrected affine shared-renderer integration"]
fn corrected_affine_refinement_matches_scalar_coverage() {
    use crate::{
        crop::Crop,
        frame_raster::FrameRaster,
        geometry::{Flips, FramePoint, PhotoOrientation, QuarterTurns, RollOrientation},
        gpu::GpuFlatField,
        processing::Pipeline,
        rendering::CpuRenderer,
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut scan = Scan::open(&PathBuf::from("samples/DSC04416.ARW"), None).unwrap();
        let a = scan.area;
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [a.x, a.y, a.width, a.height],
                [2, 2],
                vec![
                    DVec3::new(0.9, 1., 0.95),
                    DVec3::new(1.2, 1.1, 1.),
                    DVec3::new(1., 1.2, 0.9),
                    DVec3::new(1.3, 1.15, 1.1),
                ],
            )
            .unwrap(),
        );
        scan.set_flat_field(Some(field.clone())).unwrap();
        let gpu_field = Arc::new(GpuFlatField::new(&device, field).unwrap());
        let calibration = scan
            .processing_calibration(Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
                input_floor: scan.input_floor,
            })
            .unwrap();
        let roll = RollSettings::new_grade();
        let photo = PhotoSettings::default();
        let pipeline = Pipeline::new(calibration, roll, photo, scan.camera_to_ap1).unwrap();
        let parameters =
            ProcessingParameters::positive(calibration, roll, photo, scan.camera_to_ap1).unwrap();
        let renderer = RendererCache::default().get(&device, &queue).unwrap();
        let generation = AtomicU64::new(0);
        for angle in [-23.7_f64, 31.2] {
            let composition = Composition::with_crop(
                a,
                RollOrientation {
                    turns: QuarterTurns::One,
                    horizontal: true,
                    ..Default::default()
                },
                PhotoOrientation {
                    flips: Flips {
                        vertical: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Some(Crop {
                    center: [1200.25, 1200.75],
                    size: [1100, 300],
                    angle: angle.to_radians(),
                }),
            )
            .unwrap();
            let frame = FrameRaster::new(
                composition,
                FramePoint([0., 0.]),
                FramePoint(composition.size().map(f64::from)),
                [13, 9],
            )
            .unwrap();
            let bounds = frame.bounds().unwrap();
            assert!(
                bounds.width > 1024 && bounds.height > 256,
                "must exercise tile seams"
            );
            let view = ViewRequest {
                area: bounds,
                output: frame.output,
                frame: Some(frame),
            };
            let mut working = Vec::new();
            for y in bounds.y..bounds.y + bounds.height {
                for x in bounds.x..bounds.x + bounds.width {
                    working.push(pipeline.apply(scan.pixel(x, y).unwrap()).unwrap().rgb);
                }
            }
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let image = crate::refinement::refine(
                    &renderer,
                    &device,
                    (&queue, &crate::platform::SubmissionGate::default()),
                    (&scan, Some(gpu_field.clone())),
                    &[],
                    parameters,
                    target,
                    (&generation, 0),
                    view,
                )
                .unwrap()
                .unwrap();
                assert_eq!(image.processing_byte_len(), (0, 0));
                let bytes = pixels(&device, &queue, &image);
                let actual: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
                let scalar = CpuRenderer::new(target).unwrap().render(&working).unwrap();
                let mut worst = 0_f64;
                for y in 0..frame.output[1] {
                    for x in 0..frame.output[0] {
                        let coverage = frame.footprint([x, y]).unwrap();
                        let mut expected = DVec3::ZERO;
                        for ([sx, sy], weight) in coverage.pixels(bounds).unwrap() {
                            expected += scalar
                                [((sy - bounds.y) * bounds.width + sx - bounds.x) as usize]
                                .as_dvec3()
                                * weight
                                / coverage.area();
                        }
                        for c in 0..3 {
                            let error = (f64::from(actual[(y * frame.output[0] + x) as usize][c])
                                - expected[c])
                                .abs();
                            worst = worst.max(error);
                            assert!(
                                error < 2e-5,
                                "target={target:?} angle={angle} pixel={x},{y} channel={c} error={error}"
                            );
                        }
                    }
                }
                assert!(
                    crate::refinement::refine(
                        &renderer,
                        &device,
                        (&queue, &crate::platform::SubmissionGate::default()),
                        (&scan, Some(gpu_field.clone())),
                        &[],
                        parameters,
                        target,
                        (&generation, 1),
                        view
                    )
                    .unwrap()
                    .is_none()
                );
                let inconsistent = ViewRequest {
                    output: [12, 9],
                    ..view
                };
                assert!(
                    crate::refinement::refine(
                        &renderer,
                        &device,
                        (&queue, &crate::platform::SubmissionGate::default()),
                        (&scan, Some(gpu_field.clone())),
                        &[],
                        parameters,
                        target,
                        (&generation, 0),
                        inconsistent
                    )
                    .is_err()
                );
                println!(
                    "affine_refinement_integration angle={angle} target={target:?} worst_absolute_error={worst:.9e}"
                );
            }
        }
    });
}

#[test]
#[ignore = "read-only local RAW/Metal cold import prewarm measurement"]
fn initial_import_with_renderer_prewarm() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = Arc::new(RendererCache::default());
        let started = std::time::Instant::now();
        renderers.prewarm(device.clone(), queue.clone());
        let paths = [
            "DSC04397.ARW",
            "DSC04378.ARW",
            "DSC04416.ARW",
            "DSC04393.ARW",
        ]
        .map(|name| PathBuf::from("samples").join(name));
        let mut document = Document::default();
        document.import(&paths).unwrap();
        let source = document.project.source(document.project.order[2]).unwrap();
        let (ready, scan) = prepare_source(
            &renderers,
            &device,
            &queue,
            paths[2].clone(),
            Some(document.project.development.as_ref().unwrap().fixed_wb),
            Some(source),
            None,
            document.project.camera_basis.as_ref(),
        )
        .unwrap();
        assert!(ready.thumbnail.is_some());
        println!(
            "initial_import_probe prewarm=true registered_sources=4 total_seconds={:.6}",
            started.elapsed().as_secs_f64()
        );
        let directory = tempfile::tempdir().unwrap();
        let mut seeded = Thumbnails::with_disk(
            device.clone(),
            queue.clone(),
            renderers.clone(),
            Some(disk::DiskCache::at(directory.path().into())),
        );
        let (file, proxy) = ready.thumbnail.clone().unwrap();
        seeded.seed(source, &ready.development, file.clone(), proxy);
        let key = ImageKey {
            source: SourceKey::new(source, &ready.development, file),
            target: OutputTarget::DisplayP3,
            field: None,
        };
        use egui_wgpu::CallbackTrait;
        let image = ready.images[OutputTarget::DisplayP3 as usize].clone();
        let parameters = ProcessingParameters::positive(
            Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
                input_floor: scan.input_floor,
            },
            RollSettings::new_grade(),
            PhotoSettings::default(),
            scan.camera_to_ap1,
        )
        .unwrap();
        let composition = Composition::with_crop(
            ready.area,
            document.project.orientation,
            source.orientation,
            source.crop,
        )
        .unwrap();
        let scope_frame = crate::frame_raster::FrameRaster::new(
            composition,
            crate::geometry::FramePoint([0., 0.]),
            crate::geometry::FramePoint(composition.size().map(f64::from)),
            [image.linear.width(), image.linear.height()],
        )
        .unwrap();
        let settings = crate::scopes::Settings {
            target: OutputTarget::DisplayP3,
            ..Default::default()
        };
        let gpu = Arc::new(crate::gpu_scopes::GpuScopes::new(&device).await.unwrap());
        let scope = crate::gpu_scopes::ScopePaint::new(
            gpu.clone(),
            image.clone(),
            parameters,
            scope_frame,
            settings,
            egui::Context::default(),
        );
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [1100, 800],
            pixels_per_point: 1.,
        };
        let mut resources = egui_wgpu::CallbackResources::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        seeded.request(key.clone(), paths[2].clone(), source.clone(), None);
        let mut encoder = device.create_command_encoder(&Default::default());
        scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(!image.prepare(&device, &mut encoder, parameters));
        queue.submit([encoder.finish()]);
        while seeded.pending.is_some() {
            seeded.receive(&document);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(seeded.ready_count(&document, OutputTarget::DisplayP3), 1);
        wait(&device, &queue);
        let counts = gpu
            .status(&image, parameters, scope_frame, settings)
            .unwrap()
            .unwrap();
        assert!(counts.valid > 0);
        assert_eq!(
            counts.valid + counts.invalid,
            image.linear.width() * image.linear.height()
        );
        assert_eq!(gpu.reduction_count(), 1);
        let mut restarted = Thumbnails::with_disk(
            device.clone(),
            queue.clone(),
            renderers,
            Some(disk::DiskCache::at(directory.path().into())),
        );
        restarted.request(key, paths[2].clone(), source.clone(), None);
        while restarted.pending.is_some() {
            restarted.receive(&document);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(restarted.ready_count(&document, OutputTarget::DisplayP3), 1);
        let mut encoder = device.create_command_encoder(&Default::default());
        scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(!image.prepare(&device, &mut encoder, parameters));
        queue.submit([encoder.finish()]);
        wait(&device, &queue);
        assert_eq!(gpu.reduction_count(), 1);
        assert_eq!(
            gpu.status(&image, parameters, scope_frame, settings)
                .unwrap()
                .unwrap(),
            counts
        );
        println!(
            "prewarm_scope_thumbnail_integration reductions=1 counts={counts:?} selected_processing_reused=true"
        );
    });
}

#[test]
#[ignore = "read-only local RAW cold thumbnail with project-relative locator"]
fn project_relative_locator_cold_thumbnail() {
    pollster::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("inputs")).unwrap();
        std::fs::create_dir(directory.path().join("grades")).unwrap();
        let input = std::env::current_dir()
            .unwrap()
            .join("samples/DSC04393.ARW");
        std::os::unix::fs::symlink(input, directory.path().join("inputs/scan.ARW")).unwrap();
        let mut document = Document::default();
        document.path = Some(directory.path().join("grades/project.json"));
        document
            .import(&[directory.path().join("inputs/scan.ARW")])
            .unwrap();
        let id = document.project.order[0];
        document.project.source_mut(id).unwrap().path = "../inputs/scan.ARW".into();
        let source = document.project.source(id).unwrap().clone();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut thumbnails = Thumbnails::with_disk(
            device,
            queue,
            Arc::new(RendererCache::default()),
            Some(disk::DiskCache::at(directory.path().join("cache"))),
        );
        let key = ImageKey {
            source: SourceKey::new(
                &source,
                document.project.development.as_ref().unwrap(),
                FileKey::new(&document.source_path(id).unwrap()),
            ),
            target: OutputTarget::DisplayP3,
            field: None,
        };
        thumbnails.request(key, document.source_path(id).unwrap(), source, None);
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while thumbnails.pending.is_some() {
            thumbnails.receive(&document);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            thumbnails.ready_count(&document, OutputTarget::DisplayP3),
            1
        );
    });
}

#[test]
fn ap1_repair_patch_precedes_aces_without_changing_other_pixels() {
    use crate::rendering::CpuRenderer;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = RendererCache::default().get(&device, &queue).unwrap();
        let source = [[16384, 8192, 4096, 65535], [6000, 12000, 18000, 65535]];
        let target = OutputTarget::Srgb;
        let image = GpuImage::new_shared(
            &renderer,
            &device,
            2,
            1,
            &source,
            glam::DMat3::IDENTITY,
            target,
        )
        .await
        .unwrap();
        let replacement = [-0.03, 0.25, 1.4];
        let mut encoder = device.create_command_encoder(&Default::default());
        image.encode_ap1_patch(
            &device,
            &mut encoder,
            ProcessingParameters::source(),
            &[1],
            &[[replacement[0], replacement[1], replacement[2], 0.]],
        );
        queue.submit([encoder.finish()]);
        let bytes = pixels(&device, &queue, &image);
        let actual: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
        let expected = CpuRenderer::new(target)
            .unwrap()
            .render(&[
                DVec3::new(
                    source[0][0] as f64,
                    source[0][1] as f64,
                    source[0][2] as f64,
                ) / 65535.,
                DVec3::from_array(replacement.map(f64::from)),
            ])
            .unwrap();
        for (actual, expected) in actual.iter().zip(expected) {
            for channel in 0..3 {
                assert!((actual[channel] - expected[channel]).abs() < 2e-5);
            }
        }
    });
}

fn wait(device: &wgpu::Device, queue: &wgpu::Queue) {
    let submission = queue.submit([]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
}
fn pixels(device: &wgpu::Device, queue: &wgpu::Queue, image: &GpuImage) -> Vec<u8> {
    let width = image.linear.width();
    let height = image.linear.height();
    let row = (width * 16).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("performance comparison"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        image.linear.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        image.linear.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    wait(device, queue);
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    mapped
        .chunks(row as usize)
        .flat_map(|line| line[..width as usize * 16].iter().copied())
        .collect()
}

#[test]
#[ignore = "read-only native RAW thread-depth/foreground Metal contention; build native probe first"]
fn native_development_foreground_contention() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let scan = Scan::open(&PathBuf::from("samples/DSC04393.ARW"), None).unwrap();
        let (width, height, codes) = scan.preview_source(1024).unwrap();
        let image = GpuImage::new_shared(
            &renderer,
            &device,
            width,
            height,
            &codes,
            scan.camera_to_ap1,
            OutputTarget::DisplayP3,
        )
        .await
        .unwrap();
        let params = ProcessingParameters::positive(
            Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
                input_floor: scan.input_floor,
            },
            RollSettings::new_grade(),
            PhotoSettings::default(),
            scan.camera_to_ap1,
        )
        .unwrap();
        for threads in [1, 2, 4, 8, 2] {
            let finished = AtomicBool::new(false);
            let mut latencies = std::thread::scope(|scope| {
                let foreground = scope.spawn(|| {
                    let mut latencies = Vec::new();
                    while !finished.load(Ordering::Acquire) {
                        let tick = std::time::Instant::now();
                        let mut encoder = device.create_command_encoder(&Default::default());
                        image.encode(&device, &mut encoder, params);
                        queue.submit([encoder.finish()]);
                        wait(&device, &queue);
                        latencies.push(tick.elapsed().as_secs_f64());
                        std::thread::sleep(
                            Duration::from_millis(16).saturating_sub(tick.elapsed()),
                        );
                    }
                    latencies
                });
                let output = std::process::Command::new(".cache/native-deps/probe")
                    .args([
                        "samples/DSC04393.ARW",
                        "samples/DSC04397.ARW",
                        &threads.to_string(),
                        "--decode-only",
                    ])
                    .output()
                    .unwrap();
                finished.store(true, Ordering::Release);
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                println!("{}", String::from_utf8_lossy(&output.stdout));
                foreground.join().unwrap()
            });
            latencies.sort_by(f64::total_cmp);
            println!(
                "native_contention threads={threads} foreground_samples={} p95_ms={:.6} max_ms={:.6}",
                latencies.len(),
                latencies[latencies.len().saturating_sub(1) * 95 / 100] * 1000.,
                latencies.last().unwrap() * 1000.
            );
        }
    });
}

#[test]
#[ignore = "read-only local RAW/Metal import and contended refinement measurements"]
fn import_and_refinement_latency() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: crate::platform::device_limits(adapter.limits()),
                ..Default::default()
            })
            .await
            .unwrap();
        println!(
            "performance adapter={:?} budgets={:?}",
            adapter.get_info(),
            memory::CacheBudgets::current()
        );
        let paths = [
            "DSC04397.ARW",
            "DSC04378.ARW",
            "DSC04416.ARW",
            "DSC04393.ARW",
        ]
        .map(|name| PathBuf::from("samples").join(name));
        let mut document = Document::default();
        document.import(&paths).unwrap();
        let renderers = RendererCache::default();
        let source = document.project.source(document.project.order[2]).unwrap();
        let (ready, scan) = prepare_source(
            &renderers,
            &device,
            &queue,
            paths[2].clone(),
            Some(document.project.development.as_ref().unwrap().fixed_wb),
            Some(source),
            None,
            document.project.camera_basis.as_ref(),
        )
        .unwrap();
        let renderer = renderers.get(&device, &queue).unwrap();
        assert!(Arc::ptr_eq(
            &renderer,
            &renderers.get(&device, &queue).unwrap()
        ));
        let mut thumbnails = Thumbnails::new(device.clone(), queue.clone(), Arc::new(renderers));
        let (file, proxy) = ready.thumbnail.clone().unwrap();
        thumbnails.seed(source, &ready.development, file.clone(), proxy);
        let key = thumbnails::ImageKey {
            source: thumbnails::SourceKey::new(source, &ready.development, file),
            target: OutputTarget::DisplayP3,
            field: None,
        };
        thumbnails.request(key, paths[2].clone(), source.clone(), None);
        while thumbnails.pending.is_some() {
            thumbnails.receive(&document);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            thumbnails.ready_count(&document, OutputTarget::DisplayP3),
            1
        );
        let parameters = ProcessingParameters::positive(
            Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
                input_floor: scan.input_floor,
            },
            RollSettings::new_grade(),
            PhotoSettings::default(),
            scan.camera_to_ap1,
        )
        .unwrap();
        let foreground = ready.images[1].clone();
        let mut encoder = device.create_command_encoder(&Default::default());
        foreground.encode(&device, &mut encoder, parameters);
        queue.submit([encoder.finish()]);
        wait(&device, &queue);
        let view = ViewRequest {
            area: scan.area,
            output: [1538, 1026],
            frame: None,
        };
        let mut reference = None;
        for contended in [false, true] {
            for depth in [1, 2, 3, 1] {
                let finished = AtomicBool::new(false);
                let started = std::time::Instant::now();
                let (image, mut latencies) = std::thread::scope(|scope| {
                    let job = scope.spawn(|| {
                        let mut latencies = Vec::new();
                        while !finished.load(Ordering::Acquire) {
                            if contended {
                                let tick = std::time::Instant::now();
                                let mut encoder =
                                    device.create_command_encoder(&Default::default());
                                foreground.encode(&device, &mut encoder, parameters);
                                queue.submit([encoder.finish()]);
                                wait(&device, &queue);
                                latencies.push(tick.elapsed().as_secs_f64());
                                std::thread::sleep(
                                    Duration::from_millis(16).saturating_sub(tick.elapsed()),
                                );
                            } else {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                        }
                        latencies
                    });
                    let generation = AtomicU64::new(0);
                    let image = crate::refinement::refine_with_depth(
                        &renderer,
                        &device,
                        (&queue, &crate::platform::SubmissionGate::default()),
                        (&scan, None),
                        &[],
                        parameters,
                        OutputTarget::DisplayP3,
                        (&generation, 0),
                        view,
                        depth,
                    );
                    finished.store(true, Ordering::Release);
                    wait(&device, &queue);
                    (image.unwrap().unwrap(), job.join().unwrap())
                });
                let seconds = started.elapsed().as_secs_f64();
                latencies.sort_by(f64::total_cmp);
                let p95 = latencies
                    .get(latencies.len().saturating_sub(1) * 95 / 100)
                    .copied()
                    .unwrap_or(0.);
                let max = latencies.last().copied().unwrap_or(0.);
                let actual = pixels(&device, &queue, &image);
                if let Some(expected) = &reference {
                    assert!(&actual == expected, "depth changes reconstruction");
                } else {
                    reference = Some(actual);
                }
                println!(
                    "refinement_probe contended={contended} depth={depth} seconds={seconds:.6} foreground_frames={} foreground_p95_seconds={p95:.6} foreground_max_seconds={max:.6}",
                    latencies.len()
                );
            }
        }
    });
}
