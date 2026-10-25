use super::*;
use crate::renderer::RendererCache;
#[cfg(feature = "ocio-reference")]
use std::sync::atomic::AtomicU64;

#[cfg(feature = "ocio-reference")]
#[path = "editor_local_adjustment_raw_tests.rs"]
mod local_adjustments;

#[cfg(feature = "ocio-reference")]
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
        let roll = RollSettings::default();
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
fn authored_prewarm_reuses_selected_processing_scopes_and_disk_thumbnail() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = Arc::new(RendererCache::default());
        renderers.prewarm(device.clone(), queue.clone());
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
        let paths = [
            "patches.dng",
            "illuminated-patches.dng",
            "roll-0.dng",
            "roll-1.dng",
        ]
        .map(|name| root.join(name));
        let mut document = Document::default();
        document.import(&paths).unwrap();
        let source = document.project.source(document.project.order[2]).unwrap();
        let (ready, scan) = prepare_source(
            &renderers,
            &device,
            &queue,
            paths[2].clone(),
            Some(document.project.development().as_ref().unwrap().fixed_wb),
            Some(source),
            None,
        )
        .unwrap();
        assert!(ready.thumbnail.is_some());
        let directory = tempfile::tempdir().unwrap();
        let mut seeded = Thumbnails::with_disk(
            device.clone(),
            queue.clone(),
            renderers.clone(),
            Some(disk::DiskCache::at(directory.path().into())),
        );
        let (file, proxy) = ready.thumbnail.clone().unwrap();
        seeded.seed(source, file.clone(), proxy);
        let key = tests::key(&document, document.project.order[2]);
        let _ = file;
        use egui_wgpu::CallbackTrait;
        let image = ready.proxy.image(OutputTarget::DisplayP3).unwrap();
        let parameters = ProcessingParameters::positive(
            Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
                input_floor: scan.input_floor,
            },
            RollSettings::default(),
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
        let job = tests::service_job(
            &seeded,
            key.clone(),
            paths[2].clone(),
            source.clone(),
            None,
            true,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        scope.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(!image.prepare(&device, &mut encoder, parameters));
        queue.submit([encoder.finish()]);
        tests::complete_service_request(&mut seeded, &document, job);
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
        let job = tests::service_job(
            &restarted,
            key,
            paths[2].clone(),
            source.clone(),
            None,
            true,
        );
        tests::complete_service_request(&mut restarted, &document, job);
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
fn authored_project_relative_locator_develops_a_cold_thumbnail() {
    pollster::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("inputs")).unwrap();
        std::fs::create_dir(directory.path().join("grades")).unwrap();
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
        std::fs::copy(input, directory.path().join("inputs/scan.dng")).unwrap();
        let mut document = Document::default();
        document.path = Some(directory.path().join("grades/project.json"));
        document
            .import(&[directory.path().join("inputs/scan.dng")])
            .unwrap();
        let id = document.project.order[0];
        document.project.source_mut(id).unwrap().path = "../inputs/scan.dng".into();
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
        let key = tests::key(&document, id);
        let job = tests::service_job(
            &thumbnails,
            key,
            document.source_path(id).unwrap(),
            source,
            None,
            true,
        );
        tests::complete_service_request(&mut thumbnails, &document, job);
        assert_eq!(
            thumbnails.ready_count(&document, OutputTarget::DisplayP3),
            1
        );
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

#[cfg(feature = "ocio-reference")]
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
