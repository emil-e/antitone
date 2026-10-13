//! Explicit private-RAW diagnostics, not a photographic acceptance gate.
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    sources: Vec<PathBuf>,
    #[serde(default)]
    reference: Option<PathBuf>,
    #[serde(default)]
    area: Option<[u32; 4]>,
}

fn wait(editor: &mut Editor) {
    let start = std::time::Instant::now();
    while editor.busy {
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "{}",
            editor.status
        );
        std::thread::sleep(Duration::from_millis(10));
        editor.receive();
    }
}

fn write_preview(
    path: &Path,
    size: [u32; 2],
    rgb: &[[u16; 3]],
) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::create_new(path)?;
    let mut encoder = tiff::encoder::TiffEncoder::new(file)?;
    let mut image = encoder.new_image::<tiff::encoder::colortype::RGB16>(size[0], size[1])?;
    image.encoder().write_tag(
        tiff::tags::Tag::Unknown(34675),
        crate::profiles::icc(OutputTarget::Srgb),
    )?;
    image.encoder().write_tag(
        tiff::tags::Tag::Software,
        "Antitone diagnostic nearest-centre preview",
    )?;
    image.write_data(&rgb.iter().flatten().copied().collect::<Vec<_>>())?;
    Ok(())
}

fn render_roll(editor: &Editor, output: &Path) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let project = &editor.document.project;
    let calibration = project.calibration()?.ok_or("no installed calibration")?;
    let picture = rect(project.analysis_area.ok_or("no installed picture area")?);
    let wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let renderer = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?;
    let mut frames = Vec::new();
    for id in &project.order {
        let record = project.source(*id)?;
        let path = editor.document.source_path(*id)?;
        let mut scan = Scan::open(&path, Some(wb))?;
        document::verify_scan(record, &scan)?;
        if let Some(field) = project.active_flat_field() {
            scan.set_flat_field(Some(field.map.clone()))?;
        }
        let pipeline = crate::processing::Pipeline::new(
            scan.processing_calibration(calibration)?,
            project.roll,
            record.photo,
            scan.camera_to_ap1,
        )?;
        let (w, h) = scan.preview_dimensions(384)?;
        let grid = crate::geometry::SourceRaster::new(scan.area, [w, h])?;
        let mut working = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let [sx, sy] = grid.source_pixel([x, y])?;
                working.push(pipeline.apply(scan.pixel(sx, sy)?)?.rgb);
            }
        }
        let rendered = renderer.render(&working)?;
        let encoded = OutputTarget::Srgb.encode_u16(&rendered)?;
        let name = path
            .file_stem()
            .ok_or("missing source filename")?
            .to_string_lossy();
        write_preview(&output.join(format!("{name}.tif")), [w, h], &encoded)?;
        let eligibility = scan
            .analysis_samples(picture, MAX_SAMPLES_PER_FRAME, || false)?
            .ok_or("uncancellable sampling cancelled")?;
        let mut luminances: Vec<_> = eligibility
            .pixels
            .iter()
            .map(|&p| {
                pipeline
                    .apply(p)
                    .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(v.rgb))
            })
            .collect::<Result<_, _>>()?;
        luminances.sort_unstable_by(f64::total_cmp);
        let lower_tail: [f64; 3] = std::array::from_fn(|channel| {
            let mut values: Vec<_> = eligibility.pixels.iter().map(|p| p[channel]).collect();
            values.sort_unstable_by(f64::total_cmp);
            let at = (values.len() - 1) as f64 * 0.005;
            let fraction = at.fract();
            values[at.floor() as usize] * (1. - fraction) + values[at.ceil() as usize] * fraction
        });
        let global_tail_count: [usize; 3] = std::array::from_fn(|channel| {
            eligibility
                .pixels
                .iter()
                .filter(|p| p[channel] <= calibration.dmax[channel])
                .count()
        });
        frames.push(json!({
            "file": path.file_name().unwrap().to_string_lossy(),
            "local_support": scan.propose_flat_field_area()?.map(array),
            "picture_attempted": eligibility.attempted,
            "picture_accepted": eligibility.pixels.len(),
            "capture_rejected": eligibility.rejected_capture,
            "development_rejected": eligibility.rejected_development,
            "floor_rejected": eligibility.rejected_floor,
            "picture_ap1_luminance_median": luminances.get(luminances.len()/2),
            "picture_channel_lower_half_percent": lower_tail,
            "picture_count_at_or_below_roll_dmax": global_tail_count,
            "preview_negative_fraction": working.iter().filter(|p| p.min_element()<0.).count() as f64/working.len() as f64,
            "preview_hdr_fraction": working.iter().filter(|p| p.max_element()>1.).count() as f64/working.len() as f64,
        }));
        println!("rendered {}", path.display());
    }
    Ok(frames)
}

#[path = "editor_analysis_balance_diagnostics.rs"]
mod balance;
#[path = "editor_analysis_photo_diagnostics.rs"]
mod photo;

fn percentiles(values: impl IntoIterator<Item = f64>) -> Value {
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort_unstable_by(f64::total_cmp);
    if values.is_empty() {
        return Value::Null;
    }
    json!([0.01, 0.05, 0.5, 0.95, 0.99].map(|q| {
        let at = (values.len() - 1) as f64 * q;
        values[at.floor() as usize] * (1. - at.fract()) + values[at.ceil() as usize] * at.fract()
    }))
}

#[test]
#[ignore = "private RAW appearance counterfactuals; requires explicit report and fresh output"]
fn diagnose_analyzed_appearance() -> Result<(), Box<dyn std::error::Error>> {
    use crate::processing::{Pipeline, Response};
    let report_path =
        PathBuf::from(std::env::var_os("ANTITONE_ROLL_REPORT").ok_or("missing report")?);
    let output = PathBuf::from(std::env::var_os("ANTITONE_ROLL_OUTPUT").ok_or("missing output")?);
    let render_names: Vec<_> = std::env::var("ANTITONE_ROLL_FRAMES")?
        .split(',')
        .map(str::to_owned)
        .collect();
    let report: Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let project: crate::project::Project = serde_json::from_value(report["project"].clone())?;
    fs::create_dir(&output)?;
    let calibration = project.calibration()?.ok_or("no calibration")?;
    let wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let picture = rect(project.analysis_area.ok_or("no picture area")?);
    let support = report["frames"]
        .as_array()
        .ok_or("no frames")?
        .iter()
        .find_map(|f| serde_json::from_value::<[u32; 4]>(f["local_support"].clone()).ok())
        .map(rect)
        .ok_or("no diagnostic film support")?;
    assert_eq!(
        suggest(support),
        picture,
        "probe requires matching baseline support"
    );
    let matrix = glam::DMat3::from_cols_array_2d(
        &project
            .camera_basis
            .as_ref()
            .ok_or("no basis")?
            .camera_to_ap1,
    );
    let mut owned = Vec::new();
    let mut audit = Vec::new();
    let mut mean_keys = Vec::new();
    for &id in &project.order {
        let record = project.source(id)?;
        let mut scan = Scan::open(&record.path, Some(wb))?;
        document::verify_scan(record, &scan)?;
        if let Some(field) = project.active_flat_field() {
            scan.set_flat_field(Some(field.map.clone()))?;
        }
        let film = scan
            .analysis_samples(support, MAX_SAMPLES_PER_FRAME, || false)?
            .ok_or("cancelled")?;
        let pixels = scan
            .analysis_samples(picture, MAX_SAMPLES_PER_FRAME, || false)?
            .ok_or("cancelled")?;
        let pipeline = Pipeline::new(
            scan.processing_calibration(calibration)?,
            project.roll,
            record.photo,
            matrix,
        )?;
        require_picture(&pixels)?;
        let pre_photo = Pipeline::new(
            scan.processing_calibration(calibration)?,
            project.roll,
            crate::processing::PhotoSettings::default(),
            matrix,
        )?;
        let positive_luminances = pixels
            .pixels
            .iter()
            .map(|&p| {
                pre_photo
                    .apply(p)
                    .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(v.rgb))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let positive_luminances: Vec<_> = positive_luminances
            .into_iter()
            .filter(|y| *y > 1e-8)
            .collect();
        if positive_luminances.is_empty() {
            return Err("no positive exposure population".into());
        }
        let mean_key = positive_luminances.iter().sum::<f64>() / positive_luminances.len() as f64;
        mean_keys.push(mean_key);
        let luminances = pixels
            .pixels
            .iter()
            .map(|&p| {
                pipeline
                    .apply(p)
                    .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(v.rgb))
            })
            .collect::<Result<Vec<_>, _>>()?;
        audit.push(json!({
            "file": record.path.file_name().unwrap().to_string_lossy(),
            "film_samples": film.pixels.len(), "picture_samples": pixels.pixels.len(),
            "film_rgb_percentiles_1_5_50_95_99": ([0,1,2].map(|c| percentiles(film.pixels.iter().map(|p| p[c])))),
            "picture_rgb_percentiles_1_5_50_95_99": ([0,1,2].map(|c| percentiles(pixels.pixels.iter().map(|p| p[c])))),
            "dmax_tail_counts": ([0,1,2].map(|c| pixels.pixels.iter().filter(|p| p[c]<=calibration.dmax[c]).count())),
            "above_dmin_counts": ([0,1,2].map(|c| pixels.pixels.iter().filter(|p| p[c]>calibration.dmin[c]).count())),
            "pre_photo_positive_picture_mean_y": mean_key,
            "picture_mean_y": luminances.iter().sum::<f64>()/luminances.len() as f64,
            "picture_y_percentiles_1_5_50_95_99": percentiles(luminances),
        }));
        owned.push((film.pixels, pixels.pixels));
    }
    let frames: Vec<_> = owned
        .iter()
        .map(|(film, picture)| FrameSamples { film, picture })
        .collect();
    let floor = if let Some(field) = project.active_flat_field() {
        field.map.precision_floor()?
    } else {
        calibration.input_floor
    };
    let original = roll_analysis::estimate(
        &frames,
        floor,
        matrix,
        crate::processing::RollSettings {
            response: Response::Original,
            ..project.roll
        },
        || false,
    )?
    .ok_or("cancelled")?;
    let first_dmin = crate::processing::Calibration {
        dmin: crate::sampling::roll_dmin(frames[0].film, floor)?,
        ..calibration
    }
    .validate()?;
    let mut variants = vec![("baseline", project.roll, calibration)];
    for (name, delta) in [
        ("EV-minus-0.5", -0.5),
        ("EV-minus-1", -1.),
        ("EV-minus-1.5", -1.5),
    ] {
        variants.push((
            name,
            crate::processing::RollSettings {
                exposure: project.roll.exposure + delta,
                ..project.roll
            },
            calibration,
        ));
    }
    assert!((original.dmin - calibration.dmin).abs().max_element() < 1e-12);
    assert!((original.dmax - calibration.dmax).abs().max_element() < 1e-12);
    variants.push(("toe-1-reanalyzed", original.roll, calibration));
    let y_median = |roll| -> Result<f64, Error> {
        let pipeline = Pipeline::new(
            crate::processing::Calibration {
                input_floor: floor,
                ..calibration
            },
            roll,
            crate::processing::PhotoSettings::default(),
            matrix,
        )?;
        let mut y = frames[0]
            .picture
            .iter()
            .map(|&p| {
                pipeline
                    .apply(p)
                    .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(v.rgb))
            })
            .collect::<Result<Vec<_>, _>>()?;
        y.sort_unstable_by(f64::total_cmp);
        Ok(y[y.len() / 2])
    };
    let matched = crate::processing::RollSettings {
        exposure: original.roll.exposure
            + (y_median(project.roll)? / y_median(original.roll)?).log2(),
        ..original.roll
    };
    variants.push(("toe-1-matched-first-frame", matched, calibration));
    variants.push(("first-frame-only-Dmin", project.roll, first_dmin));
    let mean_key = mean_keys.iter().sum::<f64>() / mean_keys.len() as f64;
    mean_keys.sort_unstable_by(f64::total_cmp);
    let at = (mean_keys.len() - 1) as f64 / 2.;
    let median_log_mean = mean_keys[at.floor() as usize].log2() * (1. - at.fract())
        + mean_keys[at.ceil() as usize].log2() * at.fract();
    for (label, log_key) in [
        ("mean-key", mean_key.log2()),
        ("median-frame-mean-key", median_log_mean),
    ] {
        variants.push((
            label,
            crate::processing::RollSettings {
                exposure: project.roll.exposure + 0.18_f64.log2() - log_key,
                ..project.roll
            },
            calibration,
        ));
    }
    if let Ok(names) = std::env::var("ANTITONE_ROLL_VARIANTS") {
        let names: Vec<_> = names.split(',').collect();
        variants.retain(|(label, _, _)| names.contains(label));
        assert_eq!(variants.len(), names.len(), "unknown or duplicate variant");
    }
    let renderer = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?;
    let mut rendered_frames = Vec::new();
    for &id in &project.order {
        let record = project.source(id)?;
        let name = record.path.file_stem().unwrap().to_string_lossy();
        if !render_names.iter().any(|n| n == name.as_ref()) {
            continue;
        }
        let mut scan = Scan::open(&record.path, Some(wb))?;
        document::verify_scan(record, &scan)?;
        if let Some(field) = project.active_flat_field() {
            scan.set_flat_field(Some(field.map.clone()))?;
        }
        let (w, h) = scan.preview_dimensions(1024)?;
        let grid = crate::geometry::SourceRaster::new(scan.area, [w, h])?;
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let [sx, sy] = grid.source_pixel([x, y])?;
                pixels.push(scan.pixel(sx, sy)?);
            }
        }
        let mut measurements = Vec::new();
        for (label, roll, references) in &variants {
            let pipeline = Pipeline::new(
                scan.processing_calibration(*references)?,
                *roll,
                record.photo,
                matrix,
            )?;
            let working = pixels
                .iter()
                .map(|&p| pipeline.apply(p).map(|v| v.rgb))
                .collect::<Result<Vec<_>, _>>()?;
            let rendered = renderer.render(&working)?;
            write_preview(
                &output.join(format!("{name}-{label}.tif")),
                [w, h],
                &OutputTarget::Srgb.encode_u16(&rendered)?,
            )?;
            measurements.push(json!({"variant":label,"roll":roll,"dmin":references.dmin.to_array(),"preview_y_percentiles_1_5_50_95_99":percentiles(working.iter().map(|&p| crate::color::AP1_TO_XYZ.row(1).dot(p)))}));
        }
        let patches = [[100,1000,100,1000], [8420,1000,100,1000], [1000,100,6000,60]].map(|area| {
            let mean = scan.reference(rect(area));
            json!({"area":area, "mean":mean.as_ref().ok().map(|v|v.to_array()), "error":mean.err().map(|e|e.to_string())})
        });
        rendered_frames.push(json!({"file":name,"variants":measurements,"candidate_margin_patches_not_material_validated":patches}));
    }
    assert_eq!(
        rendered_frames.len(),
        render_names.len(),
        "missing requested frame"
    );
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"baseline_project":project,"audit":audit,"rendered":rendered_frames,"toe_1_proposal":{"dmin":original.dmin.to_array(),"dmax":original.dmax.to_array(),"roll":original.roll},"contract":"unchanged Pipeline and CPU ACES; diagnostic grade copies, not installed edits; 1024px nearest-centres; toe-1 is a counterfactual, not a selected response"}),
        )?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private RAW/Metal all-roll diagnostics; requires explicit manifest and fresh output"]
fn diagnose_sample_rolls() {
    let manifest =
        PathBuf::from(std::env::var_os("ANTITONE_ROLL_CASES").expect("ANTITONE_ROLL_CASES"));
    let output =
        PathBuf::from(std::env::var_os("ANTITONE_ROLL_OUTPUT").expect("ANTITONE_ROLL_OUTPUT"));
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    assert!(!cases.is_empty());
    fs::create_dir(&output).expect("choose a fresh output directory");
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue, None);
        let mut reports = Vec::new();
        for case in cases {
            assert!(!case.sources.is_empty());
            assert!(
                Path::new(&case.name)
                    .file_name()
                    .is_some_and(|name| name == case.name.as_str())
            );
            let directory = output.join(&case.name);
            fs::create_dir(&directory).unwrap();
            editor.start_document(DocumentWork::New);
            wait(&mut editor);
            editor.add_raws(case.sources.clone());
            wait(&mut editor);
            let mut prepared = editor.ready.is_some();
            if prepared && let Some(reference) = &case.reference {
                editor.begin_flat_edit(reference.clone(), None);
                wait(&mut editor);
                if editor.region.is_some() {
                    editor.apply_pick();
                    wait(&mut editor);
                    prepared = editor.correction.is_some();
                } else {
                    prepared = false;
                }
            }
            editor.document.project.analysis_area = case.area;
            let before = editor.document.project.clone();
            let started = std::time::Instant::now();
            if prepared {
                editor.begin_analysis();
                wait(&mut editor);
            }
            let installed = editor.status.starts_with("Analyzed ");
            if installed {
                for (mut after, prior) in editor
                    .document
                    .project
                    .sources
                    .clone()
                    .into_iter()
                    .zip(&before.sources)
                {
                    assert_eq!(after.photo.wb, Balance::default());
                    after.photo = prior.photo;
                    assert_eq!(&after, prior);
                }
                assert_eq!(editor.roll.gamma, before.roll.gamma);
                assert_eq!(editor.roll.response, before.roll.response);
            } else {
                assert_eq!(editor.document.project, before);
            }
            let render = if installed {
                render_roll(&editor, &directory)
            } else {
                Ok(Vec::new())
            };
            let (frames, render_error) = match render {
                Ok(frames) => (frames, None),
                Err(error) => (Vec::new(), Some(error.to_string())),
            };
            let report = json!({
                "name": case.name, "sources": case.sources, "reference": case.reference,
                "explicit_area": case.area, "status": editor.status, "installed": installed,
                "seconds_analysis_and_render": started.elapsed().as_secs_f64(),
                "project": editor.document.project, "frames": frames, "render_error": render_error,
                "preview_contract": "384px original nearest-centres; actual Pipeline and CPU ACES 100-nit sRGB; not area-averaged settled/export acceptance",
            });
            fs::write(
                directory.join("report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            println!("ROLL {}: {}", case.name, editor.status);
            reports.push(report);
            fs::write(
                output.join("reports.json"),
                serde_json::to_vec_pretty(&reports).unwrap(),
            )
            .unwrap();
        }
    });
}
