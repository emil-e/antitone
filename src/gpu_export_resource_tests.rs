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

fn exercise(
    monitor: &Arc<observation::Monitor>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), String> {
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
        let snapshot = PhotoGradeSnapshot::new(photo, vec![]).map_err(|e| e.to_string())?;
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
            let path = directory.path().join("bounded-output");
            let start = Instant::now();
            let mut checks = 0_u64;
            let mut tile_start = None;
            let mut tile_time = Duration::ZERO;
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
            monitor.phase(&format!("export-corrected-{corrected}-{format:?}"), || {
                write_checked(
                    &renderer,
                    device,
                    (queue, &gate),
                    Request {
                        scan: &scan,
                        field: gpu_field.clone(),
                        calibration,
                        roll,
                        grade: snapshot.clone(),
                        composition,
                        format,
                    },
                    &path,
                    Overwrite::Refuse,
                    || {
                        checks += 1;
                        if checks > 1 {
                            if checks.is_multiple_of(2) {
                                tile_start = Some(Instant::now());
                            } else {
                                tile_time += tile_start.take().unwrap().elapsed();
                            }
                        }
                        monitor.check().map_err(Error::Gpu)?;
                        if start.elapsed() > Duration::from_secs(180) {
                            return Err(Error::Gpu(
                                "resource diagnostic reached its three-minute export bound".into(),
                            ));
                        }
                        Ok(())
                    },
                )
            })?;
            println!(
                "export_resources file_bytes={} seconds={:.6} original_tiles={} tile_sampling_gpu_readback_seconds={:.6} reconstruction_init_encoding_seconds={:.6}",
                std::fs::metadata(&path).map_err(|e| e.to_string())?.len(),
                start.elapsed().as_secs_f64(),
                (checks - 1) / 2,
                tile_time.as_secs_f64(),
                start.elapsed().saturating_sub(tile_time).as_secs_f64()
            );
            monitor.phase("reopen-full-file-with-view-retained", || -> Result<(),Error> {
                    match format {
                        Format::Tiff(target) => {
                            let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&path).map_err(|e|Error::Export(e.to_string()))?).map_err(|e|Error::Export(e.to_string()))?;
                            assert_eq!(decoder.dimensions().unwrap(),(width,height));
                            assert_eq!(decoder.get_tag_u8_vec(tiff::tags::Tag::Unknown(34675)).unwrap(),crate::profiles::icc(target));
                            let mut samples = 0;
                            for strip in 0..decoder.strip_count().unwrap() {
                                monitor.check().map_err(Error::Gpu)?;
                                let tiff::decoder::DecodingResult::U16(pixels) = decoder.read_chunk(strip).unwrap() else { panic!("not RGB16") };
                                samples += pixels.len();
                            }
                            assert_eq!(samples,width as usize*height as usize*3);
                        }
                        Format::Exr(_) => {
                            use exr::prelude::*;
                            let image = read().no_deep_data().largest_resolution_level().rgba_channels(
                                |_,_| [0_u64;3],
                                |counts: &mut [u64;3],_,(r,g,b,a): (f32,f32,f32,f32)| {
                                    assert!([r,g,b,a].iter().all(|v|v.is_finite()));
                                    assert_eq!(a,1.);
                                    counts[0] += 1;
                                    counts[1] += u64::from(r.min(g).min(b)<0.);
                                    counts[2] += u64::from(r.max(g).max(b)>1.);
                                }
                            ).first_valid_layer().all_attributes().non_parallel().from_file(&path).map_err(|e|crate::Error::Export(e.to_string()))?;
                            assert_eq!(image.layer_data.size,Vec2(width as usize,height as usize));
                            let counts = image.layer_data.channel_data.pixels;
                            assert_eq!(counts[0],u64::from(width)*u64::from(height));
                            println!("export_resources reopened_EXR pixels={} signed_pixels={} hdr_pixels={}",counts[0],counts[1],counts[2]);
                        }
                    }
                    Ok(())
                })?;
            monitor.phase(
                "grade-retained-detail-after-export-no-original-reacquisition",
                || -> Result<(), Error> {
                    let edited = PhotoGradeSnapshot::new(
                        PhotoSettings {
                            exposure: 0.25,
                            ..photo
                        },
                        vec![],
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
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        drop(rendered);
        drop(sources);
        observation::wait(device, queue).map_err(|e| e.to_string())?;
        monitor.phase("retire-view-before-next-correction", || Ok::<_, Error>(()))?;
    }
    Ok(())
}
