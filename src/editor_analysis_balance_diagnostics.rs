//! Private gamma-balance influence tests; no installed/saved grade changes.
use super::*;
use crate::{
    color::Balance,
    processing::{Calibration, Inversion, PhotoSettings, Pipeline, RollSettings},
};
use glam::{DMat3, DVec3};

#[derive(Clone, Copy)]
enum Weighting {
    Uniform,
    Tail(f64),
    FrameMean,
}

fn quantile(y: &[f64], q: f64) -> f64 {
    let mut y = y.to_vec();
    y.sort_unstable_by(f64::total_cmp);
    let at = (y.len() - 1) as f64 * q;
    y[at.floor() as usize] * (1. - at.fract()) + y[at.ceil() as usize] * at.fract()
}

fn weights(frames: &[Vec<f64>], mode: Weighting) -> Result<Vec<f64>, &'static str> {
    if frames.is_empty()
        || frames
            .iter()
            .any(|y| y.is_empty() || y.iter().any(|v| !v.is_finite() || *v <= 0.))
    {
        return Err("invalid weighting luminances");
    }
    let mut result = Vec::new();
    for y in frames {
        let w = match mode {
            Weighting::Uniform => vec![1.; y.len()],
            Weighting::FrameMean => vec![y.len() as f64 / y.iter().sum::<f64>(); y.len()],
            Weighting::Tail(q) => {
                if !(0. < q && q < 1.) {
                    return Err("invalid tail quantile");
                }
                let cut = quantile(y, q);
                let mut w: Vec<_> = y.iter().map(|v| (cut / v).min(1.)).collect();
                let factor = y.len() as f64 / w.iter().sum::<f64>();
                for v in &mut w {
                    *v *= factor;
                }
                w
            }
        };
        result.extend(w);
    }
    Ok(result)
}

#[test]
fn joint_rgb_tail_weights_preserve_frames_and_scale() {
    let y = vec![vec![1., 2., 3., 100.], vec![10., 20., 30., 1000.]];
    let w = weights(&y, Weighting::Tail(0.5)).unwrap();
    assert!((w[..4].iter().sum::<f64>() - 4.).abs() < 1e-14);
    assert!((w[4..].iter().sum::<f64>() - 4.).abs() < 1e-14);
    for i in 0..4 {
        assert!((w[i] - w[i + 4]).abs() < 1e-14);
    }
    assert!(w[3] < w[0]);
    let rgb = DVec3::new(2., 3., 4.);
    let weighted = rgb * w[3];
    assert!((weighted.x / weighted.y - rgb.x / rgb.y).abs() < 1e-14);
    assert!(weights(&[vec![0.]], Weighting::Uniform).is_err());
    assert!(weights(&[vec![f64::NAN]], Weighting::Tail(0.99)).is_err());
    let w = weights(&y, Weighting::FrameMean).unwrap();
    for (i, f) in y.iter().enumerate() {
        assert!(
            (f.iter()
                .zip(&w[i * 4..(i + 1) * 4])
                .map(|(v, w)| v * w)
                .sum::<f64>()
                - 4.)
                .abs()
                < 1e-14
        );
    }
}

fn analysis_points(
    project: &crate::project::Project,
    cal: Calibration,
    initial: RollSettings,
    matrix: DMat3,
) -> Result<(usize, Vec<DVec3>), Box<dyn std::error::Error>> {
    let wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let picture = rect(project.analysis_area.ok_or("no area")?);
    let inversion = Inversion::new(cal, initial)?;
    let mut populations = Vec::new();
    for &id in &project.order {
        let record = project.source(id)?;
        let mut scan = Scan::open(&record.path, Some(wb))?;
        document::verify_scan(record, &scan)?;
        if let Some(field) = project.active_flat_field() {
            scan.set_flat_field(Some(field.map.clone()))?;
        }
        let samples = scan
            .analysis_samples(picture, MAX_SAMPLES_PER_FRAME, || false)?
            .ok_or("cancelled")?;
        require_picture(&samples)?;
        let mut eligible = Vec::new();
        for p in samples.pixels {
            let rgb = inversion.apply(p)?.rgb;
            let y = crate::color::AP1_TO_XYZ.row(1).dot(matrix * rgb);
            if !y.is_finite() {
                return Err(Error::NumericalOverflow.into());
            }
            if rgb.min_element() > 1e-8 && y > 1e-8 {
                eligible.push(p);
            }
        }
        if eligible.len() < 256 {
            return Err("too few positive balance samples".into());
        }
        populations.push(eligible);
    }
    let count = populations
        .iter()
        .map(Vec::len)
        .min()
        .ok_or("empty roll")?
        .min(2048);
    let points = populations
        .iter()
        .flat_map(|p| (0..count).map(move |i| p[(2 * i + 1) * p.len() / (2 * count)]))
        .collect();
    Ok((count, points))
}

#[path = "editor_analysis_tonal_diagnostics.rs"]
mod tonal;

fn fit(
    cal: Calibration,
    roll: RollSettings,
    matrix: DMat3,
    points: &[DVec3],
    weights: &[f64],
) -> Result<Balance, Error> {
    let samples: Vec<_> = points
        .iter()
        .copied()
        .zip(weights.iter().copied())
        .collect();
    crate::sampling::roll_neutral_weighted(cal, roll, &samples, matrix)
}

#[test]
#[ignore = "private gamma-balance influence test; explicit photo-exposure report and fresh output"]
fn diagnose_gamma_balance() -> Result<(), Box<dyn std::error::Error>> {
    let report_path =
        PathBuf::from(std::env::var_os("ANTITONE_ROLL_REPORT").ok_or("missing report")?);
    let output = PathBuf::from(std::env::var_os("ANTITONE_ROLL_OUTPUT").ok_or("missing output")?);
    let render_names: Vec<_> = std::env::var("ANTITONE_ROLL_FRAMES")?
        .split(',')
        .map(str::to_owned)
        .collect();
    let loo = std::env::var("ANTITONE_BALANCE_LOO").unwrap_or_else(|_| "1".into()) == "1";
    let report: Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let project: crate::project::Project =
        serde_json::from_value(report["baseline_project"].clone())?;
    let before = serde_json::to_value(&project)?;
    let roll: RollSettings = serde_json::from_value(report["roll"].clone())?;
    assert_eq!(
        RollSettings {
            exposure: project.roll.exposure,
            ..roll
        },
        project.roll
    );
    let mut cal = project.calibration()?.ok_or("no calibration")?;
    if let Some(field) = project.active_flat_field() {
        cal.input_floor = field.map.precision_floor()?;
    }
    let initial = RollSettings {
        balance: Balance::default(),
        ..roll
    };
    assert_eq!(
        initial.gamma,
        crate::processing::DEFAULT_GAMMA,
        "probe reconstructs new-grade baseline"
    );
    let matrix = DMat3::from_cols_array_2d(
        &project
            .camera_basis
            .as_ref()
            .ok_or("no basis")?
            .camera_to_ap1,
    );
    let wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let (count, points) = analysis_points(&project, cal, initial, matrix)?;
    let inversion = Inversion::new(cal, initial)?;
    let native_initial = points
        .iter()
        .map(|&p| inversion.apply(p).map(|v| v.rgb))
        .collect::<Result<Vec<_>, _>>()?;
    let luminances: Vec<_> = native_initial
        .chunks_exact(count)
        .map(|p| {
            p.iter()
                .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(matrix * *v))
                .collect()
        })
        .collect();
    let target = matrix.inverse() * DVec3::ONE;
    let native_magnitudes: Vec<Vec<f64>> = native_initial
        .chunks_exact(count)
        .map(|p| p.iter().map(|v| (*v / target).max_element()).collect())
        .collect();
    let modes = [
        ("current", Weighting::Uniform),
        ("tail99", Weighting::Tail(0.99)),
        ("tail95", Weighting::Tail(0.95)),
        ("frame-mean", Weighting::FrameMean),
        ("native99", Weighting::Tail(0.99)),
        ("native95", Weighting::Tail(0.95)),
    ];
    let mut variants = Vec::new();
    for (name, mode) in modes {
        let metric = if name.starts_with("native") {
            &native_magnitudes
        } else {
            &luminances
        };
        let w = weights(metric, mode)?;
        let balance = fit(cal, initial, matrix, &points, &w)?;
        if name == "current" {
            assert!((balance.warmth - roll.balance.warmth).abs() < 1e-8);
            assert!((balance.tint - roll.balance.tint).abs() < 1e-8);
        }
        println!("balance {name}: {balance:?}");
        variants.push((name, RollSettings { balance, ..roll }, w));
    }
    let current = Inversion::new(cal, roll)?;
    let native_current = points
        .iter()
        .map(|&p| current.apply(p).map(|v| v.rgb))
        .collect::<Result<Vec<_>, _>>()?;
    let total_initial: DVec3 = native_initial.iter().copied().sum();
    let total_current: DVec3 = native_current.iter().copied().sum();
    let mut audit = Vec::new();
    let mut omissions = Vec::new();
    for (i, &id) in project.order.iter().enumerate() {
        let record = project.source(id)?;
        let name = record.path.file_stem().ok_or("no stem")?.to_string_lossy();
        let start = i * count;
        let end = start + count;
        let a: DVec3 = native_initial[start..end].iter().copied().sum();
        let b: DVec3 = native_current[start..end].iter().copied().sum();
        let mut rank: Vec<_> = (0..count).collect();
        rank.sort_unstable_by(|&a, &b| luminances[i][b].total_cmp(&luminances[i][a]));
        let tails: Vec<_>=[0.01,0.05,0.10].into_iter().map(|fraction| {
            let n=(fraction*count as f64).ceil() as usize;
            let a_tail: DVec3=rank[..n].iter().map(|&j|native_initial[start+j]).sum();
            let b_tail: DVec3=rank[..n].iter().map(|&j|native_current[start+j]).sum();
            json!({"fraction":fraction,"pre_fit_native_channel_share":(a_tail/a).to_array(),"current_native_channel_share":(b_tail/b).to_array()})
        }).collect();
        let frame_variants=variants.iter().map(|(name,r,w)| {
            let inv=Inversion::new(cal,*r)?;
            let mut sum=DVec3::ZERO;let mut weighted=DVec3::ZERO;
            for j in start..end {let rgb=inv.apply(points[j])?.rgb;sum+=rgb;weighted+=rgb*w[j];}
            Ok(json!({"variant":name,"ap1_mean_rgb_before_exposure":(matrix*(sum/count as f64)).to_array(),"weighted_ap1_mean_rgb_before_exposure":(matrix*(weighted/w[start..end].iter().sum::<f64>())).to_array()}))
        }).collect::<Result<Vec<_>,Error>>()?;
        audit.push(json!({"file":name,"samples":count,"pre_fit_native_channel_roll_share":(a/total_initial).to_array(),"current_native_channel_roll_share":(b/total_current).to_array(),"pre_fit_ap1_mean_rgb":(matrix*(a/count as f64)).to_array(),"current_ap1_mean_rgb":(matrix*(b/count as f64)).to_array(),"brightness_ranked_tail_contribution":tails,"variants":frame_variants}));
        if loo {
            let selected: Vec<_> = points[..start]
                .iter()
                .chain(&points[end..])
                .copied()
                .collect();
            let value = match fit(cal, initial, matrix, &selected, &vec![1.; selected.len()]) {
                Ok(balance) => {
                    json!({"omitted":name,"balance":balance,"warmth_change":balance.warmth-roll.balance.warmth,"tint_change":balance.tint-roll.balance.tint})
                }
                Err(error) => json!({"omitted":name,"error":error.to_string()}),
            };
            omissions.push(value);
        }
    }
    fs::create_dir(&output)?;
    let renderer = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?;
    let mut rendered = Vec::new();
    for &id in &project.order {
        let record = project.source(id)?;
        let name = record.path.file_stem().ok_or("no stem")?.to_string_lossy();
        if !render_names.iter().any(|n| n == name.as_ref()) {
            continue;
        }
        let frame = report["frames"]
            .as_array()
            .ok_or("no exposure frames")?
            .iter()
            .find(|f| f["file"] == name.as_ref())
            .ok_or("no exposure frame")?;
        let proposal = frame["variants"]
            .as_array()
            .ok_or("no exposures")?
            .iter()
            .find(|v| v["variant"] == "photo-robust-blend99")
            .ok_or("no robust exposure")?;
        let photo: PhotoSettings = serde_json::from_value(proposal["photo"].clone())?;
        assert_eq!(
            PhotoSettings {
                exposure: record.photo.exposure,
                ..photo
            },
            record.photo
        );
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
        for (label, r, _) in &variants {
            assert_eq!(
                RollSettings {
                    balance: roll.balance,
                    ..*r
                },
                roll
            );
            let pipeline = Pipeline::new(cal, *r, photo, matrix)?;
            let working = pixels
                .iter()
                .map(|&p| pipeline.apply(p).map(|v| v.rgb))
                .collect::<Result<Vec<_>, _>>()?;
            write_preview(
                &output.join(format!("{name}-{label}.tif")),
                [w, h],
                &OutputTarget::Srgb.encode_u16(&renderer.render(&working)?)?,
            )?;
            measurements.push(json!({"variant":label,"roll":r,"photo":photo,"preview_y_percentiles_1_5_50_95_99":percentiles(working.iter().map(|p|crate::color::AP1_TO_XYZ.row(1).dot(*p)))}));
        }
        rendered.push(json!({"file":name,"variants":measurements}));
    }
    assert_eq!(
        rendered.len(),
        render_names.len(),
        "unknown or duplicate render name"
    );
    assert_eq!(serde_json::to_value(&project)?, before);
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"baseline_project":project,"fixed_roll":roll,"weighting_initial_roll":initial,"samples_per_frame":count,"variants":variants.iter().map(|(name,r,_)|json!({"variant":name,"roll":r})).collect::<Vec<_>>(),"audit":audit,"leave_one_out":omissions,"rendered":rendered,"contract":"Diagnostic only. Native original eligible points, same equal-count selection as new-grade Analyze; arithmetic-mean reconstruction asserted. Weights frozen before fitting from unbalanced AP1 Y, or max(native RGB / camera-neutral target) for native99/native95; joint RGB weights, no pixel/source changes. Tail weights min(1,p99/Y) or p95, normalized to equal frame weight; frame-mean control instead normalizes baseline frame luminance influence. Same solver, target, endpoints/global gamma/response/roll and Photo exposures; only gamma-balance coordinates vary. Leave-one-frame-out omits only diagnostic fit samples, not product sources. 1024px nearest-centre Pipeline/CPU ACES, not calibrated acceptance; original project unchanged."}),
        )?,
    )?;
    Ok(())
}
