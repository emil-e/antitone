#![cfg(all(
    feature = "raw",
    feature = "ui",
    feature = "render",
    target_os = "macos"
))]
use crate as api;
use crate::raw_test_support::train_raw;
use api::{
    color::Balance,
    crop::Crop,
    frame_raster::FrameRaster,
    geometry::{Composition, FramePoint, SourcePoint, SourceRaster, SourceRegion},
    gpu::{
        GpuPhotoGrade, ProcessingInput, ProcessingKernel, ProcessingParameters, ProcessingSource,
        SourceGeometry,
    },
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot, SpatialPipeline},
    processing::{PhotoSettings, RollSettings},
    refinement::{ViewRequest, refine},
    rendering::{CpuRenderer, OutputTarget},
    viewer::GpuImage,
};
use glam::{DVec3, Vec4};
use std::{
    sync::{Arc, atomic::AtomicU64},
    time::Instant,
};

fn locals(area: SourceRegion, count: usize, photographic: bool) -> Vec<LocalAdjustment> {
    let point = |x: f64, y: f64| {
        SourcePoint([
            f64::from(area.x) + x * f64::from(area.width),
            f64::from(area.y) + y * f64::from(area.height),
        ])
    };
    (0..count)
        .map(|i| LocalAdjustment {
            mask: if i % 2 == 0 {
                GradientMask::Linear {
                    start: point(0.15, 0.15),
                    end: point(0.85, 0.85),
                }
            } else {
                GradientMask::Radial {
                    center: point(0.5, 0.5),
                    radii: [0.43 * f64::from(area.width), 0.4 * f64::from(area.height)],
                    angle: 0.3,
                    feather: 0.8,
                }
            },
            photo: if photographic {
                PhotoSettings {
                    exposure: 0.25,
                    wb: Balance {
                        warmth: 0.005,
                        tint: -0.003,
                    },
                    contrast: 1.01,
                    contrast_colour_mix: 0.25,
                    saturation: 1.02,
                    saturation_density: 0.4,
                    ..Default::default()
                }
            } else {
                PhotoSettings {
                    exposure: if i % 2 == 0 { 0.25 } else { -0.125 },
                    ..Default::default()
                }
            },
            enabled: true,
            invert: i % 3 == 0,
            strength: 0.6,
        })
        .collect()
}
fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}
fn completion(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encode: impl FnOnce(&mut wgpu::CommandEncoder),
) -> f64 {
    let start = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    encode(&mut encoder);
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    start.elapsed().as_secs_f64() * 1000.
}
fn read_buffer(device: &wgpu::Device, queue: &wgpu::Queue, output: &wgpu::Buffer) -> Vec<Vec4> {
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: output.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(output, 0, &read, 0, output.size());
    queue.submit([encoder.finish()]);
    mapped_pixels(
        device,
        &read,
        output.size() as usize,
        output.size() as usize,
    )
}
fn read_image(device: &wgpu::Device, queue: &wgpu::Queue, image: &GpuImage) -> Vec<Vec4> {
    let texture = &image.linear;
    let row = (texture.width() * 16).div_ceil(256) * 256;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    mapped_pixels(device, &read, row as usize, texture.width() as usize * 16)
}
fn mapped_pixels(
    device: &wgpu::Device,
    read: &wgpu::Buffer,
    stride: usize,
    row_bytes: usize,
) -> Vec<Vec4> {
    let (send, recv) = std::sync::mpsc::channel();
    read.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = read.get_mapped_range(..).unwrap();
    let pixels = mapped
        .chunks_exact(stride)
        .flat_map(|row| bytemuck::cast_slice::<u8, Vec4>(&row[..row_bytes]).to_vec())
        .collect();
    drop(mapped);
    read.unmap();
    pixels
}

#[test]
#[ignore = "read-only train RAW, diagnostic calibration, Metal parity/timing and rotated local refinement; run explicitly in release"]
fn real_raw_local_preview_tiles_and_refinement() {
    let (scan, calibration) = train_raw::load();
    let area = scan.area;
    let roll = RollSettings::new_grade();
    let graded_global = PhotoSettings {
        exposure: -2.,
        contrast: 1.05,
        saturation: 1.2,
        ..Default::default()
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        println!(
            "local_raw adapter={:?} camera={} source_area={area:?} samples=3 warmups=1 timing=encode_submit_gpu_completion excludes=raw_development_upload_readback_reconstruction_scopes_presentation calibration=diagnostic",
            adapter.get_info(),
            scan.camera_model
        );
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
        let renderer = api::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let mut rasters = Vec::new();
        for (label, maximum) in [
            ("preview_1024", 1024),
            ("preview_512", 512),
            ("preview_256", 256),
        ] {
            let (width, height, pixels) = scan.preview_source(maximum).unwrap();
            rasters.push((
                label,
                SourceRaster::new(area, [width, height]).unwrap(),
                pixels,
            ));
        }
        let tile_area = SourceRegion {
            x: area.x + area.width / 2 - 512,
            y: area.y + area.height / 2 - 128,
            width: 1024,
            height: 256,
        };
        let tile = scan
            .tile_region(
                scan.area,
                tile_area.x - area.x,
                tile_area.y - area.y,
                1024,
                256,
            )
            .unwrap();
        let mut refinement_grade = None;
        rasters.push((
            "native_tile",
            SourceRaster::new(tile_area, [1024, 256]).unwrap(),
            tile,
        ));
        for (label, raster, pixels) in rasters {
            let size = raster.size();
            let region = raster.region();
            let geometry =
                SourceGeometry::new([region.x, region.y, region.width, region.height], size)
                    .unwrap();
            let mut source = ProcessingSource::new(&device, &pixels, scan.camera_to_ap1).unwrap();
            source.set_geometry(geometry).unwrap();
            assert_eq!(source.byte_len(), pixels.len() as u64 * 8);
            for i in [0, pixels.len() / 2, pixels.len() - 1] {
                let xy = raster
                    .source_pixel([i as u32 % size[0], i as u32 / size[0]])
                    .unwrap();
                assert_eq!(
                    scan.pixel(xy[0], xy[1]).unwrap(),
                    DVec3::new(
                        f64::from(pixels[i][0]),
                        f64::from(pixels[i][1]),
                        f64::from(pixels[i][2])
                    ) / 65535.
                );
            }
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("local RAW AP1"),
                size: pixels.len() as u64 * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mut global_output = Vec::new();
            for (global, count, photographic) in [
                (PhotoSettings::default(), 0, false),
                (PhotoSettings::default(), 2, false),
                (PhotoSettings::default(), 2, true),
                (PhotoSettings::default(), 3, true),
                (graded_global, 0, false),
                (graded_global, 4, true),
            ] {
                let snapshot =
                    PhotoGradeSnapshot::new(global, locals(area, count, photographic)).unwrap();
                let tick = Instant::now();
                let grade = if count == 0 {
                    None
                } else {
                    match GpuPhotoGrade::new(
                        &device,
                        calibration,
                        roll,
                        snapshot.clone(),
                        scan.camera_to_ap1,
                        area,
                    ) {
                        Ok(grade) => Some(Arc::new(grade)),
                        Err(error) => {
                            println!(
                                "local_raw path={label} global={global:?} count={count} photographic={photographic} unsupported={error}"
                            );
                            continue;
                        }
                    }
                };
                let parameters =
                    ProcessingParameters::positive(calibration, roll, global, scan.camera_to_ap1)
                        .unwrap();
                let preparation_ms = tick.elapsed().as_secs_f64() * 1000.;
                let input = grade.as_ref().map_or_else(
                    || ProcessingInput::from(parameters),
                    |grade| grade.clone().into(),
                );
                let encode = |encoder: &mut wgpu::CommandEncoder| {
                    if let Some(grade) = &grade {
                        kernel
                            .encode_grade(&device, encoder, &source, &output, grade)
                            .unwrap();
                    } else {
                        kernel.encode(&device, encoder, &source, &output, parameters);
                    }
                };
                completion(&device, &queue, encode);
                let processing_ms = median(
                    (0..3)
                        .map(|_| completion(&device, &queue, encode))
                        .collect(),
                );
                let actual = read_buffer(&device, &queue, &output);
                assert_eq!(actual.len(), pixels.len());
                if count == 0 {
                    global_output = actual.clone();
                } else {
                    assert_eq!(global_output.len(), actual.len());
                    assert!(
                        actual
                            .iter()
                            .zip(&global_output)
                            .any(|(local, global)| (local.truncate() - global.truncate())
                                .abs()
                                .max_element()
                                > 2e-5),
                        "{label}: active locals must change the real RAW output"
                    );
                }
                let pipeline =
                    SpatialPipeline::new(calibration, roll, snapshot, scan.camera_to_ap1).unwrap();
                let mut working = Vec::with_capacity(pixels.len());
                let mut worst = 0_f64;
                let mut signed = 0;
                let mut hdr = 0;
                for (i, (pixel, actual)) in pixels.iter().zip(&actual).enumerate() {
                    let xy = raster
                        .source_pixel([i as u32 % size[0], i as u32 / size[0]])
                        .unwrap();
                    let camera = DVec3::new(
                        f64::from(pixel[0]),
                        f64::from(pixel[1]),
                        f64::from(pixel[2]),
                    ) / 65535.;
                    let reference = pipeline.apply(camera, xy).unwrap();
                    let flags = (0..3).fold(0u32, |bits, c| {
                        bits | (u32::from(reference.floored[c]) << c)
                    });
                    assert_eq!(actual.w as u32, flags);
                    for c in 0..3 {
                        let ratio = (f64::from(actual[c]) - reference.rgb[c]).abs()
                            / (2e-5 + 2e-4 * reference.rgb[c].abs());
                        assert!(
                            actual[c].is_finite() && ratio <= 1.,
                            "{label} count={count} global={global:?} pixel={xy:?} actual={actual:?} reference={reference:?} ratio={ratio}"
                        );
                        worst = worst.max(ratio);
                    }
                    signed += usize::from(reference.rgb.min_element() < 0.);
                    hdr += usize::from(reference.rgb.max_element() > 1.);
                    working.push(reference.rgb);
                }
                println!(
                    "local_raw path={label} size={size:?} global={global:?} count={count} photographic={photographic} cpu_prepare_ms={preparation_ms:.3} processing_median_ms={processing_ms:.3} pixels={} signed_pixels={signed} hdr_pixels={hdr} worst_tolerance_multiple={worst:.9e} source_bytes={} working_bytes={} grade_bytes={}",
                    pixels.len(),
                    source.byte_len(),
                    output.size(),
                    grade.as_ref().map_or(0, |grade| grade.byte_len())
                );
                for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                    let mut image = GpuImage::new_shared(
                        &renderer,
                        &device,
                        size[0],
                        size[1],
                        &pixels,
                        scan.camera_to_ap1,
                        target,
                    )
                    .await
                    .unwrap();
                    image.set_geometry(geometry).unwrap();
                    assert_eq!(
                        image.processing_byte_len(),
                        (source.byte_len(), output.size())
                    );
                    completion(&device, &queue, |encoder| {
                        image.encode_processing(&device, encoder, &input).unwrap()
                    });
                    let rendered_ms = median(
                        (0..3)
                            .map(|_| {
                                completion(&device, &queue, |encoder| {
                                    image.encode_processing(&device, encoder, &input).unwrap()
                                })
                            })
                            .collect(),
                    );
                    let rendered = read_image(&device, &queue, &image);
                    let expected = CpuRenderer::new(target).unwrap().render(&working).unwrap();
                    assert_eq!(rendered.len(), expected.len());
                    let mut worst = 0_f32;
                    for (actual, expected) in rendered.iter().zip(expected) {
                        let error = (actual.truncate() - expected).abs().max_element();
                        assert!(
                            actual.is_finite() && error < 2e-5,
                            "{label} count={count} target={target:?} error={error} actual={actual:?} expected={expected:?}"
                        );
                        worst = worst.max(error);
                    }
                    println!(
                        "local_raw path={label} count={count} photographic={photographic} target={target:?} processing_aces_median_ms={rendered_ms:.3} worst_render_absolute_error={worst:.9e}"
                    );
                }
                if global == PhotoSettings::default() && count == 2 && photographic {
                    refinement_grade = grade;
                }
            }
        }
        let grade = refinement_grade
            .expect("the RAW fixture must support two photographic stages for refinement parity");
        let pipeline = SpatialPipeline::new(
            calibration,
            roll,
            grade.snapshot().clone(),
            scan.camera_to_ap1,
        )
        .unwrap();
        let composition = Composition::with_crop(
            scan.area,
            Default::default(),
            Default::default(),
            Some(Crop {
                center: [f64::from(area.width) / 2., f64::from(area.height) / 2.],
                size: [1100, 300],
                angle: 0.31,
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
            "exercise original-source geometry across tile seams"
        );
        let mut working = Vec::new();
        for y in bounds.y..bounds.y + bounds.height {
            for x in bounds.x..bounds.x + bounds.width {
                working.push(
                    pipeline
                        .apply(scan.pixel(x, y).unwrap(), [x, y])
                        .unwrap()
                        .rgb,
                );
            }
        }
        let generation = AtomicU64::new(0);
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let tick = Instant::now();
            let image = refine(
                &renderer,
                &device,
                (&queue, &crate::platform::SubmissionGate::default()),
                (&scan, None),
                grade.clone(),
                target,
                (&generation, 0),
                ViewRequest {
                    area: bounds,
                    output: frame.output,
                    frame: Some(frame),
                },
            )
            .unwrap()
            .unwrap();
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let refinement_ms = tick.elapsed().as_secs_f64() * 1000.;
            assert_eq!(image.processing_byte_len(), (0, 0));
            let actual = read_image(&device, &queue, &image);
            assert_eq!(actual.len(), (frame.output[0] * frame.output[1]) as usize);
            let rendered = CpuRenderer::new(target).unwrap().render(&working).unwrap();
            let mut worst = 0_f64;
            for y in 0..frame.output[1] {
                for x in 0..frame.output[0] {
                    let coverage = frame.footprint([x, y]).unwrap();
                    let mut expected = DVec3::ZERO;
                    for ([sx, sy], weight) in coverage.pixels(bounds).unwrap() {
                        expected += rendered
                            [((sy - bounds.y) * bounds.width + sx - bounds.x) as usize]
                            .as_dvec3()
                            * weight
                            / coverage.area();
                    }
                    let actual = actual[(y * frame.output[0] + x) as usize]
                        .truncate()
                        .as_dvec3();
                    let error = (actual - expected).abs().max_element();
                    assert!(
                        actual.is_finite() && error < 2e-5,
                        "refinement target={target:?} pixel={x},{y} actual={actual:?} expected={expected:?} error={error}"
                    );
                    worst = worst.max(error);
                }
            }
            println!(
                "local_raw_refinement target={target:?} source={}x{} output={:?} completion_ms={refinement_ms:.3} worst_absolute_error={worst:.9e}",
                bounds.width, bounds.height, frame.output
            );
        }
    });
}
