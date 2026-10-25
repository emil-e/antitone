use super::*;
use crate::processing::Calibration;

#[path = "gpu_viewport_performance_tests.rs"]
mod viewport_tests;

async fn shader_kernel(device: &wgpu::Device, shader: String) -> ProcessingKernel {
    ProcessingKernel::build_pipeline(device, shader)
        .await
        .unwrap()
}

#[derive(Debug, PartialEq)]
pub(super) struct StagePrefixKey {
    upstream: PrefixKey,
    preceding: Vec<LocalAdjustment>,
}

impl StagePrefixKey {
    pub(super) fn new(
        stamp: [u64; 2],
        source: &ProcessingSource,
        grade: &GpuPhotoGrade,
        cut: usize,
    ) -> Self {
        let geometry = match &source.flat_field {
            Some(field) => flat::FieldGeometry::new(field, source.geometry.unwrap()),
            None => flat::FieldGeometry::uncorrected(source.geometry),
        };
        Self {
            upstream: PrefixKey::new(
                stamp,
                source.camera_to_ap1,
                geometry,
                grade.parameters,
                true,
            ),
            preceding: grade.snapshot().locals()[..cut].to_vec(),
        }
    }
}

fn timed(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernel: &ProcessingKernel,
    probe: &Probe,
    source: &ProcessingSource,
    grade: &GpuPhotoGrade,
    steps: Option<&[usize]>,
) -> f64 {
    let start = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    if let Some(steps) = steps {
        probe.encode(device, &mut encoder, source, grade, steps);
    } else {
        kernel
            .encode_grade(device, &mut encoder, source, &probe.output, grade)
            .unwrap();
    }
    wait(device, queue, encoder);
    start.elapsed().as_secs_f64() * 1000.
}

fn wait(device: &wgpu::Device, queue: &wgpu::Queue, encoder: wgpu::CommandEncoder) {
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
}

fn scalar_samples(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    probe: &Probe,
    source: &ProcessingSource,
    grade: &GpuPhotoGrade,
    calibration: Calibration,
    pixels: &[[f32; 4]],
) {
    let words = read_words(device, queue, &probe.output);
    let scalar = SpatialPipeline::new(
        calibration,
        RollSettings::default(),
        grade.snapshot().clone(),
        source.camera_to_ap1,
    )
    .unwrap();
    for i in (0..pixels.len()).step_by(257) {
        let point = source.geometry.unwrap().pixel(i as u32).unwrap();
        let mut scan = DVec3::from_array(std::array::from_fn(|c| f64::from(pixels[i][c]) / 65535.));
        if let Some(field) = &source.flat_field {
            scan *= field
                .field()
                .gain(point.map(|v| f64::from(v) + 0.5))
                .unwrap();
        }
        let expected = scalar.apply(scan, point).unwrap();
        for c in 0..3 {
            let actual = f64::from(f32::from_bits(words[4 * i + c]));
            assert!(
                (actual - expected.rgb[c]).abs() <= 2e-5 + 2e-4 * expected.rgb[c].abs(),
                "stage scalar pixel={i} channel={c} actual={actual} reference={}",
                expected.rgb[c]
            );
        }
        let flags = (0..3).fold(0, |a, c| a | (u32::from(expected.floored[c]) << c));
        assert_eq!(f32::from_bits(words[4 * i + 3]), flags as f32);
    }
}

async fn investigate(
    pixels: Vec<[f32; 4]>,
    size: [u32; 2],
    area: SourceRegion,
    calibration: Calibration,
    matrix: DMat3,
    corrected: bool,
) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    println!(
        "stage_probe adapter={:?} size={size:?} corrected={corrected} samples=9 timing=encode_submit_gpu_completion excludes=compilation_upload_readback_rendering_reconstruction",
        adapter.get_info()
    );
    let kernel = ProcessingKernel::new(&device).await.unwrap();
    let probe = Probe::new(&device, pixels.len()).await;
    let geometry = SourceGeometry::new([area.x, area.y, area.width, area.height], size).unwrap();
    let mut source = ProcessingSource::new_continuous(&device, &pixels, matrix).unwrap();
    source.set_geometry(geometry).unwrap();
    let calibration = if corrected {
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [area.x, area.y, area.width, area.height],
                [2, 2],
                vec![
                    DVec3::new(1.2, 0.9, 1.1),
                    DVec3::new(0.95, 1.3, 1.05),
                    DVec3::new(1.1, 1.05, 0.88),
                    DVec3::new(1.25, 0.98, 1.15),
                ],
            )
            .unwrap(),
        );
        source
            .set_flat_field(
                Some(Arc::new(GpuFlatField::new(&device, field.clone()).unwrap())),
                geometry,
            )
            .unwrap();
        field.processing_calibration(calibration).unwrap()
    } else {
        calibration
    };
    let make = |snapshot| {
        GpuPhotoGrade::new(
            &device,
            calibration,
            RollSettings::default(),
            snapshot,
            matrix,
            area,
        )
        .unwrap()
    };
    let run = |grade: &GpuPhotoGrade, steps| {
        timed(&device, &queue, &kernel, &probe, &source, grade, steps)
    };
    let default = PhotoSettings::default();
    let photographic = photographic();
    let mut cases = Vec::new();
    for (name, photo, count) in [
        ("global", default, 0),
        (
            "exposure",
            PhotoSettings {
                exposure: 0.25,
                ..default
            },
            2,
        ),
        (
            "wb",
            PhotoSettings {
                wb: photographic.wb,
                ..default
            },
            2,
        ),
        (
            "contrast-rgb",
            PhotoSettings {
                contrast: 1.01,
                ..default
            },
            2,
        ),
        (
            "contrast-ratio",
            PhotoSettings {
                contrast: 1.01,
                contrast_colour_mix: 1.,
                ..default
            },
            2,
        ),
        (
            "contrast-interior",
            PhotoSettings {
                contrast: 1.01,
                contrast_colour_mix: 0.25,
                ..default
            },
            2,
        ),
        (
            "saturation-y",
            PhotoSettings {
                saturation: 1.02,
                ..default
            },
            2,
        ),
        (
            "saturation-max",
            PhotoSettings {
                saturation: 1.02,
                saturation_density: 1.,
                ..default
            },
            2,
        ),
        (
            "saturation-interior",
            PhotoSettings {
                saturation: 1.02,
                saturation_density: 0.4,
                ..default
            },
            2,
        ),
        ("photographic", photographic, 2),
    ] {
        let mut locals = snapshot(default, count, false).locals().to_vec();
        for local in &mut locals {
            local.photo = photo;
        }
        let grade = make(PhotoGradeSnapshot::new(default, locals).unwrap());
        run(&grade, None);
        run(&grade, None);
        scalar_samples(
            &device,
            &queue,
            &probe,
            &source,
            &grade,
            calibration,
            &pixels,
        );
        cases.push((name, grade, Vec::new()));
    }
    // Rotate fixture order so thermal drift is not confused with operator cost.
    for round in 0..9 {
        for offset in 0..cases.len() {
            let index = (round + offset) % cases.len();
            let (_, grade, times) = &mut cases[index];
            times.push(run(grade, None));
        }
    }
    for (name, _, times) in cases {
        println!(
            "stage_operator name={name} count={} median_ms={:.3}",
            usize::from(name != "global") * 2,
            median(&times)
        );
    }

    let base = snapshot(default, 3, true);
    let base_grade = make(base.clone());
    run(&base_grade, None);
    // The isolated builder uses precisely the production inversion constants.
    run(&base_grade, Some(&[0]));
    for cut in 0..3 {
        let prefix = make(PhotoGradeSnapshot::new(default, base.locals()[..cut].to_vec()).unwrap());
        run(&prefix, Some(&[6]));
        let key = StagePrefixKey::new([1, 0], &source, &base_grade, cut);
        let mut times = Vec::new();
        let mut ratios = Vec::new();
        let mut cold_times = Vec::new();
        let mut anchors = Vec::new();
        for edit in 0..9 {
            let mut locals = base.locals().to_vec();
            locals[cut].photo.saturation = 1.01 + f64::from(edit) * 0.001;
            locals[cut].strength = 0.55 + f64::from(edit) * 0.01;
            if edit % 3 == 1 {
                locals[cut].invert = !locals[cut].invert;
            }
            if edit == 3 {
                locals[cut].enabled = false;
            }
            if edit % 3 == 2 && cut + 1 < locals.len() {
                locals.swap(cut, cut + 1);
            }
            let changed = PhotoGradeSnapshot::new(default, locals).unwrap();
            assert_ne!(&changed, &base);
            // Admission always validates the complete stack, not the isolated suffix.
            let full = make(changed.clone());
            let suffix =
                make(PhotoGradeSnapshot::new(default, changed.locals()[cut..].to_vec()).unwrap());
            assert_eq!(key, StagePrefixKey::new([1, 0], &source, &full, cut));
            assert_ne!(key, StagePrefixKey::new([1, 1], &source, &full, cut));
            if cut != 0 {
                let mut upstream = changed.locals().to_vec();
                upstream[cut - 1].strength -= 0.01;
                let upstream = make(PhotoGradeSnapshot::new(default, upstream).unwrap());
                assert_ne!(key, StagePrefixKey::new([1, 0], &source, &upstream, cut));
                let mut reordered = changed.locals().to_vec();
                reordered.swap(0, cut);
                let reordered = make(PhotoGradeSnapshot::new(default, reordered).unwrap());
                assert_ne!(key, StagePrefixKey::new([1, 0], &source, &reordered, cut));
            }
            let before = run(&full, None);
            let expected = read_words(&device, &queue, &probe.output);
            let warm = run(&suffix, Some(&[5]));
            exact(
                "fresh-stage-edit",
                "suffix",
                &expected,
                &read_words(&device, &queue, &probe.output),
            );
            let cold = {
                let start = Instant::now();
                let mut encoder = device.create_command_encoder(&Default::default());
                probe.encode(&device, &mut encoder, &source, &prefix, &[6]);
                probe.encode(&device, &mut encoder, &source, &suffix, &[5]);
                wait(&device, &queue, encoder);
                start.elapsed().as_secs_f64() * 1000.
            };
            exact(
                "fresh-stage-rebuild",
                "suffix",
                &expected,
                &read_words(&device, &queue, &probe.output),
            );
            let after = run(&full, None);
            times.push(warm);
            cold_times.push(cold);
            anchors.extend([before, after]);
            ratios.push(warm / ((before + after) / 2.));
        }
        println!(
            "stage_suffix preceding={cut} total=3 fresh_edits=9 production_ms={:.3} warm_ms={:.3} paired_ratio={:.4} rebuild_plus_suffix_ms={:.3} one_prefix_bytes={} exact_pixels_per_edit={}",
            median(&anchors),
            median(&times),
            median(&ratios),
            median(&cold_times),
            probe.prefix.size(),
            pixels.len()
        );
    }
    // A source write invalidates the prefix even though response constants are reusable.
    let full = make(base.clone());
    let prefix = make(PhotoGradeSnapshot::new(default, base.locals()[..2].to_vec()).unwrap());
    let suffix = make(PhotoGradeSnapshot::new(default, base.locals()[2..].to_vec()).unwrap());
    let old_key = StagePrefixKey::new([1, 0], &source, &full, 2);
    let mut changed_pixels = pixels.clone();
    changed_pixels[0] = [65535., 0., 491., 0.];
    source.write_continuous(&queue, &changed_pixels).unwrap();
    assert_ne!(old_key, StagePrefixKey::new([1, 1], &source, &full, 2));
    run(&full, None);
    let expected = read_words(&device, &queue, &probe.output);
    scalar_samples(
        &device,
        &queue,
        &probe,
        &source,
        &full,
        calibration,
        &changed_pixels,
    );
    run(&prefix, Some(&[6]));
    run(&suffix, Some(&[5]));
    exact(
        "source-write",
        "rebuilt-suffix",
        &expected,
        &read_words(&device, &queue, &probe.output),
    );
    // Inspection must ignore positive prefix contents and local bindings.
    let mut inspection = make(base);
    inspection.parameters = ProcessingParameters::source_inspection(2.).unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    kernel.encode(
        &device,
        &mut encoder,
        &source,
        &probe.output,
        inspection.parameters,
    );
    wait(&device, &queue, encoder);
    let expected = read_words(&device, &queue, &probe.output);
    run(&inspection, Some(&[5]));
    exact(
        "inspection",
        "suffix-bypass",
        &expected,
        &read_words(&device, &queue, &probe.output),
    );
}

#[test]
#[ignore = "production-baseline operator costs and fresh stage-suffix cache experiments; no production wide cache or interaction acceptance"]
fn operators_and_stage_suffix_reuse() {
    let pixels = (0u32..SIZE[0] * SIZE[1])
        .map(|i| {
            [
                i.wrapping_mul(1871) as u16,
                i.wrapping_mul(7919) as u16,
                i.wrapping_mul(3413) as u16,
                42,
            ]
            .map(f32::from)
        })
        .collect();
    pollster::block_on(investigate(
        pixels,
        SIZE,
        area(),
        calibration(),
        camera_matrix(),
        false,
    ));
}

#[test]
#[ignore = "continuous corrected-source production-baseline operator and stage-suffix experiment"]
fn corrected_operators_and_stage_suffix_reuse() {
    let pixels = (0u32..SIZE[0] * SIZE[1])
        .map(|i| {
            [
                i.wrapping_mul(1871) as u16,
                i.wrapping_mul(7919) as u16,
                i.wrapping_mul(3413) as u16,
                42,
            ]
            .map(f32::from)
        })
        .collect();
    pollster::block_on(investigate(
        pixels,
        SIZE,
        area(),
        calibration(),
        camera_matrix(),
        true,
    ));
}

#[cfg(feature = "raw")]
#[test]
#[ignore = "read-only diagnostic RAW production operator and stage-suffix experiment"]
fn raw_operators_and_stage_suffix_reuse() {
    let (scan, calibration) = crate::raw_test_support::train_raw::load();
    let (width, height, pixels) = scan.preview_source(1024).unwrap();
    pollster::block_on(investigate(
        pixels,
        [width, height],
        scan.area,
        calibration,
        scan.camera_to_ap1,
        false,
    ));
}

#[test]
#[ignore = "exact-output complete production-lookup shader comparison; optional ANTITONE_STAGE_CANDIDATE_WGSL"]
fn paired_production_local_kernel() {
    let shader = if let Some(path) = std::env::var_os("ANTITONE_STAGE_CANDIDATE_WGSL") {
        println!(
            "production_kernel_candidate={}",
            std::path::Path::new(&path).display()
        );
        std::fs::read_to_string(path).unwrap()
    } else {
        ProcessingKernel::kernel_shader()
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let baseline = ProcessingKernel::new(&device).await.unwrap();
        let variants = [("candidate", shader_kernel(&device, shader).await)];
        let pixels: Vec<_> = (0u32..SIZE[0] * SIZE[1])
            .map(|i| {
                [
                    i.wrapping_mul(1871) as u16,
                    i.wrapping_mul(7919) as u16,
                    i.wrapping_mul(3413) as u16,
                    42,
                ]
            })
            .collect();
        let mut source = ProcessingSource::new(&device, &pixels, camera_matrix()).unwrap();
        source.set_geometry(geometry()).unwrap();
        let probe = Probe::new(&device, pixels.len()).await;
        for (case, photo) in [
            (
                "wb-only",
                PhotoSettings {
                    wb: photographic().wb,
                    ..Default::default()
                },
            ),
            ("photographic-interior", photographic()),
            (
                "photographic-rgb",
                PhotoSettings {
                    contrast_colour_mix: 0.,
                    saturation_density: 0.,
                    ..photographic()
                },
            ),
            (
                "photographic-ratio",
                PhotoSettings {
                    contrast_colour_mix: 1.,
                    saturation_density: 1.,
                    ..photographic()
                },
            ),
        ] {
            let mut locals = snapshot(PhotoSettings::default(), 2, true)
                .locals()
                .to_vec();
            for local in &mut locals {
                local.photo = photo;
            }
            let grade = GpuPhotoGrade::new(
                &device,
                calibration(),
                RollSettings::default(),
                PhotoGradeSnapshot::new(PhotoSettings::default(), locals).unwrap(),
                camera_matrix(),
                area(),
            )
            .unwrap();
            let run = |kernel: &ProcessingKernel| {
                timed(&device, &queue, kernel, &probe, &source, &grade, None)
            };
            run(&baseline);
            let expected = read_words(&device, &queue, &probe.output);
            for (name, kernel) in &variants {
                run(kernel);
                exact(
                    case,
                    name,
                    &expected,
                    &read_words(&device, &queue, &probe.output),
                );
                run(kernel);
            }
            let mut times = [Vec::new()];
            let mut ratios = [Vec::new()];
            let mut anchors = Vec::new();
            for round in 0..9 {
                let before = run(&baseline);
                for offset in 0..variants.len() {
                    let index = (round + offset) % variants.len();
                    let time = run(&variants[index].1);
                    times[index].push(time);
                }
                let after = run(&baseline);
                anchors.extend([before, after]);
                for index in 0..variants.len() {
                    ratios[index].push(times[index][round] / ((before + after) / 2.));
                }
            }
            for (index, (name, _)) in variants.iter().enumerate() {
                println!(
                    "production_kernel_variant case={case} candidate={name} production_ms={:.3} candidate_ms={:.3} paired_ratio={:.4} exact_pixels={}",
                    median(&anchors),
                    median(&times[index]),
                    median(&ratios[index]),
                    pixels.len()
                );
            }
        }
    });
}
