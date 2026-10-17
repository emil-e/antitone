//! Opt-in bounded resource observation, not pressure or physical interaction acceptance.
use super::*;
use crate::cache_observation as observation;
use crate::{
    crop::Crop,
    frame_raster::FrameRaster,
    geometry::{PhotoOrientation, QuarterTurns, RollOrientation},
    gpu::{
        ProcessingInput,
        view::{RenderedView, ViewKernels, ViewSource, ViewSourceSpec},
    },
    processing::PhotoSettings,
};
use std::time::{Duration, Instant};

#[test]
#[ignore = "agreed GPU window, private read-only RAWs, bounded full-size files and sampled process resources"]
fn full_size_export_resources_with_retained_working_view() {
    observation::with_monitor(exercise).unwrap();
}

#[derive(Clone, Copy, Debug)]
enum Backend {
    Gpu,
    PairedLru,
    Serial,
    Uncached,
    Cached,
    #[cfg(feature = "ocio-reference")]
    Reference,
}

fn exercise(
    monitor: &Arc<observation::Monitor>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), String> {
    let backends: &[Backend] = match std::env::var("ANTITONE_EXPORT_RESOURCE_BACKEND").as_deref() {
        Ok("gpu") | Err(_) => &[Backend::Gpu],
        Ok("serial") => &[Backend::Serial],
        Ok("paired-lru") => &[Backend::PairedLru],
        Ok("compare-batch") => &[Backend::Serial, Backend::PairedLru],
        Ok("compare-batch-reverse") => &[Backend::PairedLru, Backend::Serial],
        Ok("compare-forecast") => &[Backend::PairedLru, Backend::Gpu],
        Ok("compare-forecast-reverse") => &[Backend::Gpu, Backend::PairedLru],
        Ok("uncached") => &[Backend::Uncached],
        Ok("cached") => &[Backend::Cached],
        Ok("compare-cache") => &[Backend::Uncached, Backend::Gpu],
        #[cfg(feature = "ocio-reference")]
        Ok("reference") => &[Backend::Reference],
        #[cfg(feature = "ocio-reference")]
        Ok("compare") => &[Backend::Reference, Backend::Gpu],
        _ => return Err("unsupported export diagnostic backend; see docs/diagnostics.md".into()),
    };
    println!("export_resources backends={backends:?} note=reference_is_not_a_production_fallback");
    let root = std::path::PathBuf::from(
        std::env::var_os("ANTITONE_RAW_SAMPLES").ok_or("set absolute ANTITONE_RAW_SAMPLES")?,
    );
    if !root.is_absolute() {
        return Err("sample path must be absolute".into());
    }
    for name in ["DSC04378.ARW", "DSC04397.ARW", "DSC04524.ARW"] {
        let raw = crate::raw::RawFile::open(&root.join(name)).map_err(|e| e.to_string())?;
        observation::source_bound(&raw.metadata)?;
    }
    let (mut scan, calibration) = monitor.phase("load-read-only-native-source", || {
        Ok::<_, Error>(crate::raw_test_support::train_raw::load())
    })?;
    let native = scan
        .source()
        .retained_bytes()
        .ok_or("native accounting unavailable")?;
    println!(
        "export_resources original={}x{} native_source_bytes={native} source_leases={}",
        scan.area.width,
        scan.area.height,
        Arc::strong_count(scan.source())
    );
    let gate = SubmissionGate::default();
    let renderer = monitor.phase("renderer-infrastructure", || {
        pollster::block_on(GpuRenderer::new_with_submissions(
            device,
            queue,
            gate.clone(),
        ))
    })?;
    #[cfg(feature = "ocio-reference")]
    if backends.iter().any(|b| matches!(b, Backend::Reference)) {
        monitor.phase("reference-infrastructure", || {
            crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?.render(&[DVec3::splat(0.18)])
        })?;
    }
    let kernels = monitor.phase("view-infrastructure", || {
        pollster::block_on(ViewKernels::new(device))
    })?;
    let directory =
        tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache"))
            .map_err(|e| e.to_string())?;
    let context = egui::Context::default();
    let cases = match std::env::var("ANTITONE_EXPORT_RESOURCE_CASE").as_deref() {
        Ok("corrected-native") => vec![(true, false)],
        Ok("rotated") => vec![(false, true)],
        Ok("native") => vec![(false, false)],
        Ok("corrected-rotated") => vec![(true, true)],
        Err(_) => vec![(false, false), (true, true)],
        _ => return Err("unknown export resource case".into()),
    };
    let tiff_only = match std::env::var("ANTITONE_EXPORT_RESOURCE_FORMAT").as_deref() {
        Ok("tiff") => true,
        Ok("both") | Err(_) => false,
        _ => return Err("unknown export resource format".into()),
    };
    for (corrected, rotated) in cases {
        let area = scan.area;
        let field = corrected.then(|| {
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [area.x, area.y, area.width, area.height],
                    [2, 2],
                    vec![
                        DVec3::new(0.8, 0.9, 1.),
                        DVec3::new(1.2, 1.1, 1.05),
                        DVec3::new(1., 1.1, 0.9),
                        DVec3::new(1.1, 0.95, 1.2),
                    ],
                )
                .unwrap(),
            )
        });
        scan.set_flat_field(field.clone())
            .map_err(|e| e.to_string())?;
        let gpu_field = field
            .as_ref()
            .map(|f| GpuFlatField::new(device, f.clone()).map(Arc::new))
            .transpose()
            .map_err(|e| e.to_string())?;
        let adjusted = scan
            .processing_calibration(calibration)
            .map_err(|e| e.to_string())?;
        let roll = RollSettings::new_grade();
        let photo = PhotoSettings {
            contrast: 0.8,
            saturation: 1.1,
            ..Default::default()
        };
        let locals = match std::env::var("ANTITONE_EXPORT_RESOURCE_LOCALS").as_deref() {
            Err(_) | Ok("0") => vec![],
            Ok("2") => resource_locals(area),
            _ => return Err("resource locals must be0 or2".into()),
        };
        println!("export_resources ordered_locals={}", locals.len());
        let snapshot = PhotoGradeSnapshot::new(photo, locals).map_err(|e| e.to_string())?;
        let input: ProcessingInput = Arc::new(
            GpuPhotoGrade::new(
                device,
                adjusted,
                roll,
                snapshot.clone(),
                scan.camera_to_ap1,
                area,
            )
            .map_err(|e| e.to_string())?,
        )
        .into();
        let composition = Composition::with_crop(
            area,
            RollOrientation {
                turns: if rotated {
                    QuarterTurns::One
                } else {
                    QuarterTurns::Zero
                },
                ..Default::default()
            },
            PhotoOrientation::default(),
            rotated.then_some(Crop {
                center: [
                    area.x as f64 + area.width as f64 / 2.,
                    area.y as f64 + area.height as f64 / 2.,
                ],
                size: [area.width - 512, area.height - 512],
                angle: 0.015,
            }),
        )
        .map_err(|e| e.to_string())?;
        let mut frame = FrameRaster::full(composition).map_err(|e| e.to_string())?;
        let max_side = frame.output[0].max(frame.output[1]) as f64;
        frame.output = frame
            .output
            .map(|n| (n as f64 * 3072. / max_side).ceil() as u32);
        let spec = ViewSourceSpec::new(1, frame, adjusted, roll, scan.camera_to_ap1, field)
            .map_err(|e| e.to_string())?;
        let underlay_spec = spec.underlay().map_err(|e| e.to_string())?;
        let underlay_size = underlay_spec.frame.output;
        let sources = monitor
            .phase(
                &format!("acquire-detail-underlay-corrected-{corrected}"),
                || {
                    ViewSource::acquire_many(
                        device,
                        queue,
                        &gate,
                        renderer.clone(),
                        kernels.clone(),
                        &scan,
                        gpu_field.clone(),
                        vec![spec, underlay_spec],
                        || monitor.check().is_err(),
                    )
                },
            )?
            .ok_or("source acquisition stopped")?;
        let mut rendered = Vec::new();
        for source in &sources {
            let view = RenderedView::new(
                device,
                source.clone(),
                adjusted,
                roll,
                snapshot.clone(),
                &input,
                OutputTarget::Srgb,
                None,
            )
            .map_err(|e| e.to_string())?;
            view.ensure_submitted(device, queue, &gate, &context);
            rendered.push(view);
        }
        observation::wait(device, queue).map_err(|e| e.to_string())?;
        println!(
            "export_resources corrected={corrected} rotated={rotated} synthetic_field_2x2={corrected} detail={:?} underlay={underlay_size:?} retained_view_raster_bytes={} native_source_bytes={native} note=declared_bytes_are_not_added_to_RSS_or_Metal",
            frame.output,
            (u64::from(frame.output[0]) * u64::from(frame.output[1])
                + u64::from(underlay_size[0]) * u64::from(underlay_size[1]))
                * 16
                * 3
        );
        for format in [
            Format::Tiff(OutputTarget::Srgb),
            Format::Exr(ExrPrecision::Full),
        ] {
            if tiff_only && matches!(format, Format::Exr(_)) {
                continue;
            }
            let compare_outputs = backends.len() > 1
                && backends.iter().all(|b| {
                    matches!(
                        b,
                        Backend::Gpu
                            | Backend::PairedLru
                            | Backend::Serial
                            | Backend::Uncached
                            | Backend::Cached
                    )
                });
            let mut expected_digest = None;
            for &backend in backends {
                let path = directory.path().join("bounded-output");
                let start = Instant::now();
                let mut checks = 0_u64;
                let mut tile_start = None;
                let mut tile_time = Duration::ZERO;
                let mut job_stats = (0_u64, 0_u64, 0_usize);
                #[cfg(feature = "ocio-reference")]
                let mut reference_samples = 0;
                #[cfg(not(feature = "ocio-reference"))]
                let reference_samples = 0;
                let [width, height] = composition.size();
                println!(
                    "export_resources begin corrected={corrected} format={format:?} output={width}x{height} encoded_frame_bytes={} source_leases={}",
                    u64::from(width)
                        * u64::from(height)
                        * if matches!(format, Format::Tiff(_)) {
                            6
                        } else {
                            12
                        },
                    Arc::strong_count(scan.source())
                );
                let label = format!("export-{backend:?}-corrected-{corrected}-{format:?}");
                monitor.phase(&label, || {
                    let request = Request {
                        look: None,
                        scan: &scan,
                        field: gpu_field.clone(),
                        calibration,
                        roll,
                        grade: snapshot.clone(),
                        repairs: &[],
                        composition,
                        format,
                    };
                    let stop = || {
                        monitor.check().map_err(Error::Gpu)?;
                        if start.elapsed() > Duration::from_secs(180) {
                            return Err(Error::Export(
                                "resource diagnostic reached its three-minute export bound".into(),
                            ));
                        }
                        Ok(())
                    };
                    match backend {
                        Backend::Gpu
                        | Backend::PairedLru
                        | Backend::Serial
                        | Backend::Uncached
                        | Backend::Cached => {
                            let stage_start = Instant::now();
                            let (result, timings) = crate::export_timing::record(|| {
                                let cached = matches!(backend, Backend::Cached)
                                    || (matches!(
                                        backend,
                                        Backend::Gpu | Backend::PairedLru | Backend::Serial
                                    ) && cache_eligible(composition));
                                let mut check = || {
                                    checks += 1;
                                    if checks > 1 {
                                        if checks.is_multiple_of(2) {
                                            tile_start = Some(Instant::now());
                                        } else {
                                            tile_time += tile_start.take().unwrap().elapsed();
                                        }
                                    }
                                    stop()
                                };
                                if matches!(backend, Backend::Serial) {
                                    super::pair_tests::write_single(
                                        &renderer,
                                        device,
                                        (queue, &gate),
                                        request,
                                        (&path, Overwrite::Refuse),
                                        cached,
                                        &mut check,
                                    )
                                } else if matches!(backend, Backend::PairedLru) {
                                    write_job_with_policy(
                                        &renderer,
                                        device,
                                        (queue, &gate),
                                        request,
                                        (&path, Overwrite::Refuse),
                                        if cached {
                                            CachePolicy::Lru
                                        } else {
                                            CachePolicy::None
                                        },
                                        &mut check,
                                    )
                                } else {
                                    write_job(
                                        &renderer,
                                        device,
                                        (queue, &gate),
                                        request,
                                        (&path, Overwrite::Refuse),
                                        cached,
                                        &mut check,
                                    )
                                }
                            });
                            println!(
                                "export_stage backend={backend:?} format={format:?} complete={}",
                                result.is_ok()
                            );
                            timings.report(stage_start.elapsed());
                            result.map(|stats| job_stats = stats)
                        }
                        #[cfg(feature = "ocio-reference")]
                        Backend::Reference => {
                            reference_samples = write_reference(request, &path, stop)?;
                            Ok(())
                        }
                    }
                })?;
                let source_size = tile_size(area);
                let gpu_samples =
                    job_stats.1 * u64::from(source_size[0]) * u64::from(source_size[1]);
                println!(
                    "export_resources backend={backend:?} reference_sample_evaluations={reference_samples} gpu_sample_evaluations={gpu_samples}"
                );
                println!(
                    "export_resources file_bytes={} seconds={:.6} tile_requests={} original_tiles={} cache_hits={} peak_cached_payload_bytes={} tile_access_seconds={:.6} other_job_seconds={:.6}",
                    std::fs::metadata(&path).map_err(|e| e.to_string())?.len(),
                    start.elapsed().as_secs_f64(),
                    job_stats.0,
                    job_stats.1,
                    job_stats.0.saturating_sub(job_stats.1),
                    job_stats.2,
                    tile_time.as_secs_f64(),
                    start.elapsed().saturating_sub(tile_time).as_secs_f64()
                );
                monitor.phase(
                    "reopen-full-file-with-view-retained",
                    || -> Result<(), Error> {
                        match format {
                            Format::Tiff(target) => {
                                let mut decoder = tiff::decoder::Decoder::new(
                                    std::fs::File::open(&path).map_err(|e| Error::Export(e.to_string()))?,
                                )
                                .map_err(|e| Error::Export(e.to_string()))?;
                                assert_eq!(decoder.dimensions().unwrap(), (width, height));
                                assert_eq!(
                                    decoder
                                        .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                                        .unwrap(),
                                    crate::profiles::icc(target)
                                );
                                let mut samples = 0;
                                for strip in 0..decoder.strip_count().unwrap() {
                                    monitor.check().map_err(Error::Gpu)?;
                                    let tiff::decoder::DecodingResult::U16(pixels) =
                                        decoder.read_chunk(strip).unwrap()
                                    else {
                                        panic!("not RGB16")
                                    };
                                    samples += pixels.len();
                                }
                                assert_eq!(samples, width as usize * height as usize * 3);
                            }
                            Format::Exr(_) => {
                                use exr::prelude::*;
                                let image = read()
                                    .no_deep_data()
                                    .largest_resolution_level()
                                    .rgba_channels(
                                        |_, _| [0_u64; 3],
                                        |counts: &mut [u64; 3], _, (r, g, b, a): (f32, f32, f32, f32)| {
                                            assert!([r, g, b, a].iter().all(|v| v.is_finite()));
                                            assert_eq!(a, 1.);
                                            counts[0] += 1;
                                            counts[1] += u64::from(r.min(g).min(b) < 0.);
                                            counts[2] += u64::from(r.max(g).max(b) > 1.);
                                        },
                                    )
                                    .first_valid_layer()
                                    .all_attributes()
                                    .non_parallel()
                                    .from_file(&path)
                                    .map_err(|e| crate::Error::Export(e.to_string()))?;
                                assert_eq!(image.layer_data.size, Vec2(width as usize, height as usize));
                                let counts = image.layer_data.channel_data.pixels;
                                assert_eq!(counts[0], u64::from(width) * u64::from(height));
                                println!(
                                    "export_resources reopened_EXR pixels={} signed_pixels={} hdr_pixels={}",
                                    counts[0], counts[1], counts[2]
                                );
                            }
                        }
                        Ok(())
                    },
                )?;
                monitor.phase(
                    "grade-retained-detail-after-export-no-original-reacquisition",
                    || -> Result<(), Error> {
                        let edited = PhotoGradeSnapshot::new(
                            PhotoSettings {
                                exposure: 0.25,
                                ..photo
                            },
                            snapshot.locals().to_vec(),
                        )?;
                        let edited_input: ProcessingInput = Arc::new(GpuPhotoGrade::new(
                            device,
                            adjusted,
                            roll,
                            edited.clone(),
                            scan.camera_to_ap1,
                            area,
                        )?)
                        .into();
                        let updated = RenderedView::new(
                            device,
                            sources[0].clone(),
                            adjusted,
                            roll,
                            edited,
                            &edited_input,
                            OutputTarget::Srgb,
                            Some(&rendered[0]),
                        )?;
                        updated.ensure_submitted(device, queue, &gate, &context);
                        observation::wait(device, queue).map_err(|e| Error::Gpu(e.to_string()))?;
                        assert!(!updated.in_flight());
                        assert_eq!(updated.processing, edited_input.signature());
                        assert_eq!(rendered[0].processing, input.signature());
                        Ok(())
                    },
                )?;
                if compare_outputs && matches!(format, Format::Tiff(_)) {
                    let digest =
                        monitor.phase("compare-complete-gpu-files", || -> Result<_, Error> {
                            use std::io::Read;
                            let mut file = std::fs::File::open(&path)
                                .map_err(|e| Error::Export(e.to_string()))?;
                            let mut hash = blake3::Hasher::new();
                            let mut buffer = vec![0u8; 1024 * 1024];
                            loop {
                                let n = file
                                    .read(&mut buffer)
                                    .map_err(|e| Error::Export(e.to_string()))?;
                                if n == 0 {
                                    break;
                                }
                                hash.update(&buffer[..n]);
                            }
                            Ok(hash.finalize())
                        })?;
                    if let Some(expected) = expected_digest {
                        assert_eq!(digest, expected, "complete GPU files differ");
                        println!("export_batch complete_file_equal=true digest={digest}");
                    } else {
                        expected_digest = Some(digest);
                    }
                }
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        drop(rendered);
        drop(sources);
        observation::wait(device, queue).map_err(|e| e.to_string())?;
        monitor.phase("retire-view-before-next-correction", || Ok::<_, Error>(()))?;
    }
    Ok(())
}

fn resource_locals(
    area: crate::geometry::SourceRegion,
) -> Vec<crate::local_adjustments::LocalAdjustment> {
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment},
    };
    let [x, y] = [area.x as f64, area.y as f64];
    let [width, height] = [area.width as f64, area.height as f64];
    vec![
        LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([x, y]),
                end: SourcePoint([x + width, y + height]),
            },
            photo: PhotoSettings {
                exposure: 0.5,
                wb: crate::color::Balance {
                    warmth: 0.003,
                    tint: -0.002,
                },
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.7,
        },
        LocalAdjustment {
            mask: GradientMask::Radial {
                center: SourcePoint([x + width / 2., y + height / 2.]),
                radii: [width / 3., height / 3.],
                angle: 0.3,
                feather: 0.4,
            },
            photo: PhotoSettings {
                exposure: -0.25,
                contrast: 1.05,
                saturation: 1.08,
                ..Default::default()
            },
            enabled: true,
            invert: true,
            strength: 0.6,
        },
    ]
}

#[cfg(feature = "ocio-reference")]
#[test]
fn reference_benchmark_counts_samples_and_preserves_bounded_stop_checks() {
    let scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let calibration = crate::processing::Calibration {
        dmin: DVec3::splat(3000. / 4031.),
        dmax: DVec3::splat(128. / 4031.),
        input_floor: scan.input_floor,
    };
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let request = || Request {
        look: None,
        scan: &scan,
        field: None,
        calibration,
        roll: RollSettings::new_grade(),
        grade: PhotoGradeSnapshot::new(PhotoSettings::default(), vec![]).unwrap(),
        repairs: &[],
        composition,
        format: Format::Tiff(OutputTarget::Srgb),
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reference.tif");
    let samples = write_reference(request(), &path, || Ok(())).unwrap();
    assert_eq!(
        samples,
        u64::from(scan.area.width) * u64::from(scan.area.height)
    );
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
    std::fs::remove_file(&path).unwrap();
    let mut checks = 0;
    let result = write_reference(request(), &path, || {
        checks += 1;
        if checks == 3 {
            Err(Error::Export("diagnostic stop".into()))
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err(Error::Export("diagnostic stop".into())));
    assert_eq!(checks, 3);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[cfg(feature = "ocio-reference")]
fn write_reference(
    request: Request<'_>,
    path: &std::path::Path,
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<u64, Error> {
    let Request {
        scan,
        calibration,
        roll,
        grade,
        repairs,
        composition,
        format,
        look,
        ..
    } = request;
    let metadata = export::ProcessingMetadata::with_grade(roll.response, &grade);
    let pipeline = crate::local_adjustments::SpatialPipeline::new(
        scan.processing_calibration(calibration)?,
        roll,
        grade,
        scan.camera_to_ap1,
    )?;
    let mut samples = 0_u64;
    check()?;
    let pixel = |x, y| {
        if samples.is_multiple_of(4096) {
            check()?;
        }
        samples += 1;
        let sample = scan.pixel(x, y)?;
        let value = if let Some(repair) = repairs
            .iter()
            .find(|repair| repair.support().contains([x, y]))
        {
            let mut pixel = crate::processing::Pipeline::ungraded(
                scan.processing_calibration(calibration)?,
                roll,
                scan.camera_to_ap1,
            )?
            .apply(sample)?;
            pixel.rgb = glam::DVec3::from_array(repair.pixel([x, y], [0.; 3]).map(f64::from));
            pipeline.grade_neutral(pixel, [x, y])?.rgb
        } else {
            pipeline.apply(sample, [x, y])?.rgb
        };
        if matches!(format, Format::Tiff(_)) {
            look.as_ref()
                .map_or(Ok(value), |asset| asset.lut().apply(value))
        } else {
            Ok(value)
        }
    };
    match format {
        Format::Tiff(target) => export::tiff_composed(
            path,
            composition,
            target,
            Overwrite::Refuse,
            metadata,
            pixel,
        )?,
        Format::Exr(precision) => export::exr_composed(
            path,
            composition,
            precision,
            Overwrite::Refuse,
            metadata,
            pixel,
        )?,
    }
    check()?;
    Ok(samples)
}
