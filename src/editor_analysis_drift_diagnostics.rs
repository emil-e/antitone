//! WB-invariant gamma objective and pre/post-WB diagnostic copies only.
use super::*;
use glam::DVec2;
use std::time::Instant;

type Bands = Vec<[Vec<DVec3>; 3]>;

fn bands(
    cal: Calibration,
    initial: RollSettings,
    matrix: DMat3,
    count: usize,
    points: &[DVec3],
) -> Result<Bands, Error> {
    let inv = Inversion::new(cal, initial)?;
    points
        .chunks_exact(count)
        .map(|frame| {
            let y = frame
                .iter()
                .map(|&p| {
                    inv.apply(p)
                        .map(|v| crate::color::AP1_TO_XYZ.row(1).dot(matrix * v.rgb))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(
                [(0.01, 0.33), (0.33, 0.67), (0.67, 0.99)].map(|(low, high)| {
                    band_indices(&y, count, low, high, false)
                        .iter()
                        .map(|&i| frame[i])
                        .collect()
                }),
            )
        })
        .collect()
}

fn drift(ap1_means: [DVec3; 3], wb: Balance) -> Result<[DVec2; 2], Error> {
    let cones = crate::color::ap1_to_cones() * crate::color::wb_matrix(wb)?;
    let mut z = [DVec2::ZERO; 3];
    for (i, mean) in ap1_means.into_iter().enumerate() {
        let c = cones * mean;
        if !c.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        if c.min_element() <= 0. {
            return Err(Error::UnusableSample(
                "tonal band mean has nonpositive Bradford responses",
            ));
        }
        z[i] = crate::color::opponent_coordinates(c.ln());
    }
    Ok([z[0] - z[1], z[2] - z[1]])
}

fn evaluate(
    cal: Calibration,
    roll: RollSettings,
    matrix: DMat3,
    bands: &Bands,
) -> Result<(f64, [DVec2; 2]), Error> {
    let inv = Inversion::new(cal, roll)?;
    let mut total = [DVec2::ZERO; 2];
    for frame in bands {
        let mut means = [DVec3::ZERO; 3];
        for (i, pixels) in frame.iter().enumerate() {
            means[i] = matrix
                * crate::sampling::weighted_mean_iter(
                    pixels.iter().map(|&p| inv.apply(p).map(|v| (v.rgb, 1.))),
                )?;
        }
        let d = drift(means, Balance::default())?;
        for i in 0..2 {
            total[i] += d[i];
        }
    }
    for d in &mut total {
        *d /= bands.len() as f64;
    }
    let score = total.iter().map(|v| v.length_squared()).sum::<f64>();
    if !score.is_finite() {
        return Err(Error::NumericalOverflow);
    }
    Ok((score, total))
}

// Derivative-free search of the two existing balance coordinates, not a new curve model.
fn search(
    cal: Calibration,
    roll: RollSettings,
    matrix: DMat3,
    bands: &Bands,
) -> Result<(RollSettings, Value), Error> {
    let mut best = roll;
    let (mut score, _) = evaluate(cal, best, matrix, bands)?;
    let start_score = score;
    let mut step = 0.125;
    let mut evaluations = 1;
    let mut rejected_bounds = 0;
    let mut rejected_cones = 0;
    while step > 1e-7 {
        let base = best;
        let mut improved = false;
        for dx in [-1., 0., 1.] {
            for dy in [-1., 0., 1.] {
                if dx == 0. && dy == 0. {
                    continue;
                }
                let balance = Balance {
                    warmth: base.balance.warmth + step * dx,
                    tint: base.balance.tint + step * dy,
                };
                if balance.warmth.abs().max(balance.tint.abs()) > 1. {
                    rejected_bounds += 1;
                    continue;
                }
                let candidate = RollSettings { balance, ..base };
                evaluations += 1;
                let value = match evaluate(cal, candidate, matrix, bands) {
                    Ok((v, _)) => v,
                    Err(Error::UnusableSample(
                        "tonal band mean has nonpositive Bradford responses",
                    )) => {
                        rejected_cones += 1;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if value < score {
                    score = value;
                    best = candidate;
                    improved = true;
                }
            }
        }
        if !improved {
            step *= 0.5;
        }
        if evaluations > 4096 {
            return Err(Error::UnusableSample("gamma drift search did not converge"));
        }
    }
    Ok((
        best,
        json!({"start":roll.balance,"start_score":start_score,"balance":best.balance,"score":score,"evaluations":evaluations,"rejected_balance_bounds":rejected_bounds,"rejected_nonpositive_band_cones":rejected_cones,"resolution":step}),
    ))
}

fn optimize(
    cal: Calibration,
    roll: RollSettings,
    matrix: DMat3,
    bands: &Bands,
) -> Result<(RollSettings, Vec<Value>), Error> {
    let spans = (cal.dmin / cal.dmax).ln();
    let xy =
        crate::color::opponent_coordinates((spans / (spans.x * spans.y * spans.z).cbrt()).ln());
    let seeds = [
        roll.balance,
        Balance::default(),
        Balance {
            warmth: xy.x,
            tint: xy.y,
        }
        .validate()?,
    ];
    let mut results = Vec::new();
    let mut winner = roll;
    let mut score = f64::INFINITY;
    for seed in seeds {
        let (candidate, report) = search(
            cal,
            RollSettings {
                balance: seed,
                ..roll
            },
            matrix,
            bands,
        )?;
        let value = report["score"].as_f64().ok_or(Error::NumericalOverflow)?;
        if value < score {
            winner = candidate;
            score = value;
        }
        results.push(report);
    }
    Ok((winner, results))
}

#[test]
fn gamma_drift_recovers_known_curves_and_ignores_wb_and_exposure() {
    let cal = Calibration {
        dmin: DVec3::new(0.14, 0.098, 0.094),
        dmax: DVec3::new(0.017, 0.0052, 0.0029),
        input_floor: 1. / 65535.,
    };
    let truth = RollSettings {
        balance: Balance {
            warmth: -0.35,
            tint: 0.09,
        },
        ..RollSettings::new_grade()
    };
    let initial = RollSettings {
        balance: Balance::default(),
        ..truth
    };
    let matrix = DMat3::from_cols(
        DVec3::new(1.1, 0.03, -0.07),
        DVec3::new(0.2, 0.8, -0.1),
        DVec3::new(-0.03, 0.08, 1.3),
    );
    let mut points = Vec::new();
    for wb in [
        Balance::default(),
        Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        Balance {
            warmth: -0.15,
            tint: 0.1,
        },
    ] {
        let direction = matrix.inverse() * crate::color::wb_matrix(wb).unwrap() * DVec3::ONE;
        assert!(direction.min_element() > 0.);
        for i in 0..90 {
            let p = forward(
                cal,
                truth,
                direction * 2_f64.powf(-7. + i as f64 / 89. * 6.8),
            )
            .unwrap();
            assert!(p.min_element() > cal.input_floor);
            points.push(p);
        }
    }
    let groups = bands(cal, initial, matrix, 90, &points).unwrap();
    let (score, _) = evaluate(cal, truth, matrix, &groups).unwrap();
    assert!(score < 1e-24);
    let (fitted, searches) = optimize(cal, initial, matrix, &groups).unwrap();
    assert!((fitted.balance.warmth - truth.balance.warmth).abs() < 2e-6);
    assert!((fitted.balance.tint - truth.balance.tint).abs() < 2e-6);
    assert!(
        searches
            .iter()
            .all(|s| s["score"].as_f64().unwrap() < 1e-12)
    );
    let means = [
        DVec3::new(0.03, 0.02, 0.018),
        DVec3::new(0.2, 0.18, 0.13),
        DVec3::new(0.8, 0.9, 1.1),
    ];
    let baseline = drift(means, Balance::default()).unwrap();
    for wb in [
        Balance {
            warmth: 0.6,
            tint: -0.3,
        },
        Balance {
            warmth: -0.5,
            tint: 0.4,
        },
    ] {
        let shifted = drift(means.map(|v| v * 8.), wb).unwrap();
        for i in 0..2 {
            assert!((baseline[i] - shifted[i]).length() < 1e-12);
        }
    }
    assert!(drift([DVec3::ZERO; 3], Balance::default()).is_err());
}

#[test]
#[ignore = "private gamma-drift optimization; explicit photo-exposure report and fresh output"]
fn diagnose_gamma_drift() -> Result<(), Box<dyn std::error::Error>> {
    let input: Value = serde_json::from_slice(&fs::read(PathBuf::from(
        std::env::var_os("ANTITONE_ROLL_REPORT").ok_or("missing report")?,
    ))?)?;
    let output = PathBuf::from(std::env::var_os("ANTITONE_ROLL_OUTPUT").ok_or("missing output")?);
    let names: Vec<_> = std::env::var("ANTITONE_ROLL_FRAMES")?
        .split(',')
        .map(str::to_owned)
        .collect();
    let project: crate::project::Project =
        serde_json::from_value(input["baseline_project"].clone())?;
    let before = serde_json::to_value(&project)?;
    let roll: RollSettings = serde_json::from_value(input["roll"].clone())?;
    let initial = RollSettings {
        balance: Balance::default(),
        ..roll
    };
    let mut cal = project.calibration()?.ok_or("no calibration")?;
    if let Some(field) = project.active_flat_field() {
        cal.input_floor = field.map.precision_floor()?;
    }
    let matrix = DMat3::from_cols_array_2d(
        &project
            .camera_basis
            .as_ref()
            .ok_or("no basis")?
            .camera_to_ap1,
    );
    let (count, points) = analysis_points(&project, cal, initial, matrix)?;
    let original_fit = crate::sampling::roll_neutral(cal, initial, &points, matrix)?;
    assert!((original_fit.warmth - roll.balance.warmth).abs() < 1e-8);
    assert!((original_fit.tint - roll.balance.tint).abs() < 1e-8);
    let groups = bands(cal, initial, matrix, count, &points)?;
    let (original_score, _) = evaluate(cal, roll, matrix, &groups)?;
    let started = Instant::now();
    let (optimized, searches) = optimize(cal, roll, matrix, &groups)?;
    let optimizer_seconds = started.elapsed().as_secs_f64();
    let (optimized_score, residual) = evaluate(cal, optimized, matrix, &groups)?;
    println!(
        "gamma drift {original_score:.9} -> {optimized_score:.9}; {:?}; {optimizer_seconds:.3}s",
        optimized.balance
    );
    assert!(optimized_score <= original_score);
    assert_eq!(
        RollSettings {
            balance: roll.balance,
            ..optimized
        },
        roll
    );
    let inv = Inversion::new(cal, optimized)?;
    let means: Vec<_> = points
        .chunks_exact(count)
        .map(|p| {
            crate::sampling::weighted_mean_iter(
                p.iter()
                    .map(|&p| inv.apply(p).map(|v| (matrix * v.rgb, 1.))),
            )
        })
        .collect::<Result<_, _>>()?;
    let roll_wb = crate::color::neutral_wb(crate::sampling::mean(&means)?);
    let photo_wbs: Vec<_> = means
        .iter()
        .map(|&mean| crate::color::neutral_wb(mean))
        .collect();
    let wb_value = |value: &Result<Balance, Error>| match value {
        Ok(wb) => json!({"balance":wb}),
        Err(error) => json!({"error":error.to_string()}),
    };
    let mut audit = Vec::new();
    for (i, &id) in project.order.iter().enumerate() {
        let mut band_means = [DVec3::ZERO; 3];
        for (k, pixels) in groups[i].iter().enumerate() {
            band_means[k] = crate::sampling::weighted_mean_iter(
                pixels
                    .iter()
                    .map(|&p| inv.apply(p).map(|v| (matrix * v.rgb, 1.))),
            )?;
        }
        let d = drift(band_means, Balance::default())?;
        for wb in [roll_wb.as_ref().ok(), photo_wbs[i].as_ref().ok()]
            .into_iter()
            .flatten()
        {
            let after = drift(band_means, *wb)?;
            for k in 0..2 {
                assert!((d[k] - after[k]).length() < 1e-11);
            }
        }
        audit.push(json!({"file":project.source(id)?.path.file_stem().ok_or("no stem")?.to_string_lossy(),"pre_WB_ap1_mean":means[i].to_array(),"pre_WB_band_means":band_means.map(|v| v.to_array()),"shadow_minus_mid":d[0].to_array(),"highlight_minus_mid":d[1].to_array(),"photo_WB":wb_value(&photo_wbs[i])}));
    }
    fs::create_dir(&output)?;
    let renderer = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?;
    let fixed_wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let mut rendered = Vec::new();
    for (i, &id) in project.order.iter().enumerate() {
        let record = project.source(id)?;
        let name = record.path.file_stem().ok_or("no stem")?.to_string_lossy();
        if !names.iter().any(|n| n == name.as_ref()) {
            continue;
        }
        let f = input["frames"]
            .as_array()
            .ok_or("no frames")?
            .iter()
            .find(|f| f["file"] == name.as_ref())
            .ok_or("no exposure frame")?;
        let v = f["variants"]
            .as_array()
            .ok_or("no variants")?
            .iter()
            .find(|v| v["variant"] == "photo-robust-blend99")
            .ok_or("no robust exposure")?;
        let photo: PhotoSettings = serde_json::from_value(v["photo"].clone())?;
        assert_eq!(photo.wb, Balance::default());
        let mut scan = Scan::open(&record.path, Some(fixed_wb))?;
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
        let mut variants = Vec::new();
        for (label, r, wb) in [
            ("current", roll, Ok(Balance::default())),
            ("optimized-pre-WB", optimized, Ok(Balance::default())),
            ("optimized-roll-WB", optimized, roll_wb.clone()),
            ("optimized-photo-WB", optimized, photo_wbs[i].clone()),
        ] {
            let wb = match wb {
                Ok(wb) => wb,
                Err(error) => {
                    variants.push(json!({"variant":label,"error":error.to_string()}));
                    continue;
                }
            };
            let candidate = PhotoSettings { wb, ..photo };
            assert_eq!(
                PhotoSettings {
                    wb: photo.wb,
                    ..candidate
                },
                photo
            );
            let pipe = Pipeline::new(cal, r, candidate, matrix)?;
            let working = pixels
                .iter()
                .map(|&p| pipe.apply(p).map(|v| v.rgb))
                .collect::<Result<Vec<_>, _>>()?;
            write_preview(
                &output.join(format!("{name}-{label}.tif")),
                [w, h],
                &OutputTarget::Srgb.encode_u16(&renderer.render(&working)?)?,
            )?;
            variants.push(json!({"variant":label,"roll":r,"photo":candidate,"preview_y_percentiles_1_5_50_95_99":percentiles(working.iter().map(|p|crate::color::AP1_TO_XYZ.row(1).dot(*p)))}));
        }
        rendered.push(json!({"file":name,"variants":variants}));
    }
    assert_eq!(rendered.len(), names.len());
    assert_eq!(serde_json::to_value(&project)?, before);
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"baseline_project":project,"fixed_roll":roll,"optimized_roll":optimized,"original_score":original_score,"optimized_score":optimized_score,"residual":[residual[0].to_array(),residual[1].to_array()],"searches":searches,"optimizer_seconds":optimizer_seconds,"samples_per_frame":count,"roll_WB":wb_value(&roll_wb),"audit":audit,"rendered":rendered,"contract":"WB-invariant log-cone tonal drift, mean across frames BEFORE squared norm. Original pixels with frozen unbalanced-Y per-frame 1–33/33–67/67–99% bands. All sources retained. Existing curve model, endpoints, global gamma/response, geometry and roll/Photo EV fixed. Three deterministic pattern-search starts, balance domain -1..1, resolution <=1e-7; undefined positive-cone domain reported/rejected, other errors propagate. Gamma score is a scene-statistical proxy, not measured film response. WB fitted separately to positive-sample arithmetic means: equal-count whole-roll or individual frame; out-of-range/unusable WB explicitly fails without fallback/clamp. Common WB is applied to Photo COPIES only, no roll-state/schema/UI addition or manual-grade edits. Original project unchanged. Signed/HDR working data, original samples, native development and ACES unchanged. 1024px nearest-centre CPU comparisons, not settled/export/calibrated-colour acceptance."}),
        )?,
    )?;
    Ok(())
}
