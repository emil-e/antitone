//! Original, uncorrected camera-code windows. This path never requests crop predictions.
use super::*;
use crate::{
    project::SourceDevelopment, raw::RawFile, rendering::OutputTarget, scan::Scan,
    source_revision::FileKey,
};
use image::{Rgb, RgbImage};

struct Original {
    locator: PathBuf,
    revision: FileKey,
}

impl Original {
    fn new(locator: PathBuf, raw_root: &Path) -> Result<Self> {
        let revision = FileKey::new(&locator);
        if !revision.path.starts_with(raw_root) {
            return Err("source is outside explicit ANTITONE_RAW_ROOT".into());
        }
        let original = Self { locator, revision };
        original.verify()?;
        Ok(original)
    }

    fn verify(&self) -> Result<()> {
        if !self.revision.current_at(&self.locator) {
            return Err(
                format!("source locator or file changed: {}", self.locator.display()).into(),
            );
        }
        Ok(())
    }
}

fn verify_development(expected: &SourceDevelopment, scan: &Scan) -> Result<()> {
    let (width, height) = scan.dimensions()?;
    let area = scan.area;
    let actual = SourceDevelopment {
        dimensions: [width, height],
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.input_floor,
    };
    if *expected != actual {
        return Err("original development differs from the recorded source".into());
    }
    Ok(())
}

fn write_window(scan: &Scan, region: SourceRegion, prefix: &Path) -> Result<()> {
    let image = scan.source().camera_image()?;
    let mut bytes = Vec::with_capacity(region.width as usize * region.height as usize * 6);
    let mut display = RgbImage::new(region.width * 4, region.height * 4);
    for y in 0..region.height {
        for x in 0..region.width {
            let code = image.pixel_codes(region.x + x, region.y + y)?;
            let mut encoded = [0; 3];
            for channel in 0..3 {
                bytes.extend_from_slice(&code[channel].to_le_bytes());
                encoded[channel] = (OutputTarget::Srgb.encode(f64::from(code[channel]) / 65535.)?
                    * 255.)
                    .round() as u8;
            }
            for dy in 0..4 {
                for dx in 0..4 {
                    display.put_pixel(x * 4 + dx, y * 4 + dy, Rgb(encoded));
                }
            }
        }
    }
    fs::write(prefix.with_extension("rgb16"), bytes)?;
    display.save(prefix.with_extension("png"))?;
    Ok(())
}

fn capture(
    cases: &[CaseInput],
    inputs: &Inputs,
    raw_root: &Path,
    out: &Path,
    monitor: &crate::cache_observation::Monitor,
) -> Result<()> {
    // Validate every requested path/runtime before creating output or opening a native source.
    let mut originals = vec![];
    for case in cases {
        if case.spec.windows.is_empty() {
            return Err("capture requires independently supplied windows for every case".into());
        }
        case.project.require_runtime(&crate::raw::version())?;
        let locator = case
            .project
            .resolve_path(case.spec.source, &case.project_path)?;
        originals.push(Original::new(locator, raw_root)?);
    }
    fs::create_dir(out)?;
    let started = std::time::Instant::now();
    let checkpoint = || -> Result<()> {
        monitor.check()?;
        if started.elapsed().as_secs() >= 120 {
            return Err("original capture deadline exceeded".into());
        }
        Ok(())
    };
    let mut reports = vec![];
    for (case_index, (case, original)) in cases.iter().zip(&originals).enumerate() {
        checkpoint()?;
        original.verify()?;
        let wb = case
            .project
            .development
            .as_ref()
            .ok_or("fixed development WB unavailable")?
            .fixed_wb;
        let scan = monitor.phase("crop-quality-original", || {
            let raw = RawFile::open(&original.revision.path).map_err(|e| e.to_string())?;
            crate::cache_observation::source_bound(&raw.metadata)?;
            Scan::from_raw(raw, Some(wb)).map_err(|e| e.to_string())
        })?;
        checkpoint()?;
        verify_development(&case.project.source(case.spec.source)?.development, &scan)?;
        let mut windows = vec![];
        for (index, &window) in case.spec.windows.iter().enumerate() {
            let region: SourceRegion = window.into();
            checkpoint()?;
            let filename = format!(
                "case-{case_index}-source-{}-window-{index}",
                case.spec.source.0
            );
            write_window(&scan, region, &out.join(&filename))?;
            windows.push(json!({"source_region":window,"rgb16":format!("{filename}.rgb16"),"png":format!("{filename}.png")}));
        }
        reports.push(json!({
            "project":case.project_path,
            "source":case.spec.source,
            "source_locator":original.locator,
            "source_revision":original.revision,
            "fixed_wb":wb,
            "windows":windows,
        }));
    }
    inputs.verify()?;
    for original in originals {
        original.verify()?;
    }
    checkpoint()?;
    let report = json!({
        "prediction_free":true,
        "inputs":inputs.0,
        "display":"4x nearest neighbour; sRGB transfer on camera channels only, unity exposure, no colour matrix, inversion, correction, repairs, grading or detector overlays. PNG is a viewing aid, not a rendered photograph.",
        "rgb16":"Little-endian interleaved camera RGB16, original source orientation and pixel centres region origin + index + 0.5; no complete-original Rust copy.",
        "cases":reports,
    });
    fs::write(
        out.join("capture.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn original_freshness_checks_the_locator_after_file_or_parent_symlink_retargeting() {
    use std::os::unix::fs::symlink;
    for parent_alias in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::create_dir(root.join("first")).unwrap();
        fs::create_dir(root.join("second")).unwrap();
        fs::write(root.join("first/source.ARW"), b"first pixels").unwrap();
        fs::write(root.join("second/source.ARW"), b"other pixels").unwrap();
        let alias = root.join("alias");
        let locator = if parent_alias {
            symlink("first", &alias).unwrap();
            alias.join("source.ARW")
        } else {
            symlink("first/source.ARW", &alias).unwrap();
            alias.clone()
        };
        let original = Original::new(locator, &root).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(
            if parent_alias {
                "second"
            } else {
                "second/source.ARW"
            },
            &alias,
        )
        .unwrap();
        assert!(
            original.revision.current_at(&original.revision.path),
            "old canonical target is unchanged"
        );
        assert!(
            original.verify().is_err(),
            "retargeted project locator cannot certify a capture"
        );
    }
}

#[test]
#[ignore = "Explicit <=3 project RAWs and bounded independent windows; unchanged native resource guard, private lossless/display outputs"]
fn capture_original_crop_windows() -> Result<()> {
    let manifest =
        PathBuf::from(std::env::var_os("ANTITONE_CROP_MANIFEST").ok_or("ANTITONE_CROP_MANIFEST")?);
    let raw_root = PathBuf::from(std::env::var_os("ANTITONE_RAW_ROOT").ok_or("ANTITONE_RAW_ROOT")?)
        .canonicalize()?;
    let mut inputs = Inputs::default();
    let (cases, _) = load(&manifest, &mut inputs)?;
    let out = output()?;
    crate::cache_observation::with_monitor(|monitor, _, _| {
        capture(&cases, &inputs, &raw_root, &out, monitor).map_err(|e| e.to_string())
    })?;
    Ok(())
}
