//! Optional read-only RAW/Metal diagnostic of the actual Editor Analyze transaction.
//! Takes an explicit case manifest and fresh output directory; writes JSON and 384px sRGB TIFF previews.
//! Asserts transaction invariants, not successful analysis of every case or photographic quality.
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

fn wait(editor: Editor, start: impl FnOnce(&mut Editor, &UiContext<'_, Editor>)) -> Editor {
    crate::editor::tests::complete_analysis_operation(editor, Duration::from_secs(300), start)
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
        let mut editor = Editor::new(&device, &queue);
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
            editor = crate::editor::tests::complete_document_operation(
                editor,
                Duration::from_secs(300),
                |editor, cx| editor.start_document(Preparation::New, cx),
            );
            editor = crate::editor::tests::complete_document_operation(
                editor,
                Duration::from_secs(300),
                |editor, cx| editor.add_raws(case.sources.clone(), cx),
            );
            let mut prepared = editor.document.visit.ready.is_some();
            if prepared && let Some(reference) = &case.reference {
                editor = crate::editor::tests::complete_reference_operation(
                    editor,
                    Duration::from_secs(300),
                    |e, cx| e.begin_flat_edit(reference.clone(), None, cx),
                );
                if editor.region.is_some() {
                    editor = crate::editor::tests::complete_reference_operation(
                        editor,
                        Duration::from_secs(300),
                        |e, cx| e.apply_flat_edit(cx),
                    );
                    prepared = editor.correction.is_some();
                } else {
                    prepared = false;
                }
            }
            editor.document.project.analysis_area = case.area;
            let before = editor.document.project.clone();
            let started = std::time::Instant::now();
            if prepared {
                editor = wait(editor, |editor, cx| editor.begin_analysis(cx));
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
