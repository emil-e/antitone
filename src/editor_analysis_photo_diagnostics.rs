//! Private exposure-only counterfactuals, never installed in an editor/project.

use super::*;
use crate::processing::{PhotoSettings, Pipeline};

fn photo_keys(mut y: Vec<f64>) -> Result<(f64, f64, f64, f64), &'static str> {
    if y.is_empty() || y.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err("invalid positive luminance population");
    }
    y.sort_unstable_by(f64::total_cmp);
    let quantile = |q: f64| {
        let at = (y.len() - 1) as f64 * q;
        y[at.floor() as usize] * (1. - at.fract()) + y[at.ceil() as usize] * at.fract()
    };
    let p99 = quantile(0.99);
    // Bound only the statistic's upper tail; render the original signed/HDR pixels.
    let capped_mean = y.iter().map(|v| v.min(p99)).sum::<f64>() / y.len() as f64;
    Ok((
        y.iter().sum::<f64>() / y.len() as f64,
        quantile(0.95),
        p99,
        capped_mean,
    ))
}

fn photo_deltas((mean, p95, p99, capped_mean): (f64, f64, f64, f64)) -> [(&'static str, f64); 6] {
    let average = (0.18 / mean).log2();
    let high95 = (1. / p95).log2();
    let high99 = (1. / p99).log2();
    [
        ("roll-only", 0.),
        ("photo-mean", average),
        ("photo-highlight99", high99),
        ("photo-blend95", (average + high95) / 2.),
        ("photo-blend99", (average + high99) / 2.),
        (
            "photo-robust-blend99",
            ((0.18 / capped_mean).log2() + high99) / 2.,
        ),
    ]
}

#[test]
fn photo_key_algebra_and_population_validation() {
    let y = (1..=101).map(|v| v as f64).collect();
    assert_eq!(photo_keys(y).unwrap(), (51., 96., 100., 5150. / 101.));
    assert!(photo_keys(Vec::new()).is_err());
    assert!(photo_keys(vec![f64::NAN]).is_err());
    assert!(photo_keys(vec![0.]).is_err());
    let keys = (0.136, 0.392, 1.357, 0.13);
    let deltas = photo_deltas(keys);
    assert_eq!(deltas[0].1, 0.);
    assert!((keys.0 * 2_f64.powf(deltas[1].1) - 0.18).abs() < 1e-14);
    assert!((keys.2 * 2_f64.powf(deltas[2].1) - 1.).abs() < 1e-14);
    assert!(((keys.0 * keys.2).sqrt() * 2_f64.powf(deltas[4].1) - 0.18_f64.sqrt()).abs() < 1e-14);
    let scaled = photo_deltas((keys.0 * 4., keys.1 * 4., keys.2 * 4., keys.3 * 4.));
    for i in 1..6 {
        assert!((scaled[i].1 - deltas[i].1 + 2.).abs() < 1e-14);
    }
    let mut y = vec![0.18; 8192];
    let before = photo_deltas(photo_keys(y.clone()).unwrap());
    y[0] = 65536.;
    let after = photo_deltas(photo_keys(y).unwrap());
    assert!((after[4].1 - before[4].1).abs() > 2.);
    assert!((after[5].1 - before[5].1).abs() < 1e-14);
}

#[test]
#[ignore = "private per-photo exposure counterfactuals; explicit candidate report and fresh output"]
fn diagnose_photo_exposures() -> Result<(), Box<dyn std::error::Error>> {
    let report_path =
        PathBuf::from(std::env::var_os("ANTITONE_ROLL_REPORT").ok_or("missing report")?);
    let output = PathBuf::from(std::env::var_os("ANTITONE_ROLL_OUTPUT").ok_or("missing output")?);
    let names: Vec<_> = std::env::var("ANTITONE_ROLL_FRAMES")?
        .split(',')
        .map(str::to_owned)
        .collect();
    let selected = std::env::var("ANTITONE_PHOTO_VARIANTS").unwrap_or_else(|_| {
        photo_deltas((1., 1., 1., 1.))
            .map(|(name, _)| name)
            .join(",")
    });
    let selected: Vec<_> = selected.split(',').collect();
    assert!(!selected.is_empty());
    assert_eq!(
        photo_deltas((1., 1., 1., 1.))
            .iter()
            .filter(|(name, _)| selected.contains(name))
            .count(),
        selected.len(),
        "unknown or duplicate variant"
    );
    let report: Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let project: crate::project::Project =
        serde_json::from_value(report["baseline_project"].clone())?;
    let project_before = serde_json::to_value(&project)?;
    let first = report["rendered"]
        .as_array()
        .ok_or("no rendered frames")?
        .first()
        .ok_or("empty rendered frames")?;
    let candidate = first["variants"]
        .as_array()
        .ok_or("no variants")?
        .iter()
        .find(|v| v["variant"] == "median-frame-mean-key")
        .ok_or("no candidate roll")?;
    let roll: crate::processing::RollSettings = serde_json::from_value(candidate["roll"].clone())?;
    assert_eq!(
        crate::processing::RollSettings {
            exposure: project.roll.exposure,
            ..roll
        },
        project.roll
    );
    let calibration = project.calibration()?.ok_or("no calibration")?;
    let wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let matrix = glam::DMat3::from_cols_array_2d(
        &project
            .camera_basis
            .as_ref()
            .ok_or("no basis")?
            .camera_to_ap1,
    );
    let picture = rect(project.analysis_area.ok_or("no picture area")?);
    let renderer = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?;
    fs::create_dir(&output)?;
    let mut frames = Vec::new();
    for &id in &project.order {
        let record = project.source(id)?;
        let name = record
            .path
            .file_stem()
            .ok_or("no file stem")?
            .to_string_lossy();
        if !names.iter().any(|n| n == name.as_ref()) {
            continue;
        }
        let mut scan = Scan::open(&record.path, Some(wb))?;
        document::verify_scan(record, &scan)?;
        if let Some(field) = project.active_flat_field() {
            scan.set_flat_field(Some(field.map.clone()))?;
        }
        let calibration = scan.processing_calibration(calibration)?;
        let samples = scan
            .analysis_samples(picture, MAX_SAMPLES_PER_FRAME, || false)?
            .ok_or("cancelled")?;
        require_picture(&samples)?;
        let baseline = Pipeline::new(calibration, roll, record.photo, matrix)?;
        let y = samples
            .pixels
            .iter()
            .map(|&p| {
                baseline
                    .apply(p)
                    .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(v.rgb))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let positive: Vec<_> = y.iter().copied().filter(|v| *v > 1e-8).collect();
        if positive.len() < 512 {
            return Err("too few positive exposure samples".into());
        }
        let keys = photo_keys(positive.clone())?;
        let deltas = photo_deltas(keys);
        // An extreme hot positive exposes mean sensitivity without modifying source data.
        let mut contaminated = positive;
        contaminated[0] = keys.2 * 2_f64.powi(16);
        let stressed = photo_deltas(photo_keys(contaminated)?);
        let (w, h) = scan.preview_dimensions(1024)?;
        let grid = crate::geometry::SourceRaster::new(scan.area, [w, h])?;
        let mut pixels = Vec::new();
        for yy in 0..h {
            for xx in 0..w {
                let [sx, sy] = grid.source_pixel([xx, yy])?;
                pixels.push(scan.pixel(sx, sy)?);
            }
        }
        let mut variants = Vec::new();
        for ((label, delta), (_, stress_delta)) in deltas.into_iter().zip(stressed) {
            if !selected.contains(&label) {
                continue;
            }
            let photo = PhotoSettings {
                exposure: record.photo.exposure + delta,
                ..record.photo
            };
            assert_eq!(
                PhotoSettings {
                    exposure: record.photo.exposure,
                    ..photo
                },
                record.photo
            );
            let pipeline = Pipeline::new(calibration, roll, photo, matrix)?;
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
            variants.push(json!({"variant":label, "delta_ev":delta, "photo":photo, "single_hot_positive_delta_ev_change":stress_delta-delta, "picture_mean_y": keys.0 * 2_f64.powf(delta), "picture_p99_y":keys.2 * 2_f64.powf(delta), "preview_y_percentiles_1_5_50_95_99":percentiles(working.iter().map(|p| crate::color::AP1_TO_XYZ.row(1).dot(*p)))}));
        }
        println!(
            "photo exposure {name}: mean {:.4}, p95 {:.4}, p99 {:.4}",
            keys.0, keys.1, keys.2
        );
        frames.push(json!({"file":name, "eligible_count":samples.pixels.len(), "positive_count":y.iter().filter(|v| **v>1e-8).count(), "candidate_picture_y_percentiles_1_5_50_95_99":percentiles(y), "keys":{"mean":keys.0,"p95":keys.1,"p99":keys.2,"upper_p99_capped_mean":keys.3},"variants":variants}));
    }
    assert_eq!(
        frames.len(),
        names.len(),
        "unknown or duplicate requested frame"
    );
    assert_eq!(serde_json::to_value(&project)?, project_before);
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"baseline_project":project,"roll":roll,"frames":frames,"contract":"Diagnostic copies only; candidate roll fixed; Photo EV offsets only, all other settings preserved. Positive original eligible picture luminances. Mean target .18 and percentile target 1 are uncalibrated AP1 scene-linear priors, not display white or clipping limits; 50/50 blend in EV; robust blend caps the statistic's top 1% at p99, never working pixels. 1024px nearest-centre Pipeline/CPU ACES. One-hot-positive stress: replace first positive Y with 65536*p99; post-fit sensitivity, not capture simulation."}),
        )?,
    )?;
    Ok(())
}
