//! Tonal-population and existing-operator controls, not a new processing model.
use super::*;

#[path = "editor_analysis_drift_diagnostics.rs"]
mod drift;

fn forward(cal: Calibration, roll: RollSettings, positive: DVec3) -> Result<DVec3, Error> {
    let (toe, shoulder) = roll.response.shape()?;
    let gamma = roll.channel_gammas()?;
    Ok(DVec3::from_array(std::array::from_fn(|c| {
        let span = (cal.dmin[c] / cal.dmax[c]).ln();
        let anchor = toe * span / (gamma[c] - shoulder * span / std::f64::consts::LN_10);
        let u = (positive[c].powf(toe) * anchor.exp_m1()).ln_1p();
        let density = gamma[c] * u / (toe + shoulder * u / std::f64::consts::LN_10);
        cal.dmin[c] * (-density).exp()
    })))
}

fn band_indices(y: &[f64], count: usize, low: f64, high: f64, relative: bool) -> Vec<usize> {
    let ranked = |offset: usize, values: &[f64]| {
        let mut ids: Vec<_> = (0..values.len()).collect();
        ids.sort_unstable_by(|&a, &b| values[a].total_cmp(&values[b]));
        ids[(low * values.len() as f64).floor() as usize
            ..(high * values.len() as f64).floor() as usize]
            .iter()
            .map(|i| offset + i)
            .collect::<Vec<_>>()
    };
    if relative {
        y.chunks_exact(count)
            .enumerate()
            .flat_map(|(i, p)| ranked(i * count, p))
            .collect()
    } else {
        ranked(0, y)
    }
}

#[test]
fn known_neutral_ramp_recovers_one_gamma_fit_across_tones() {
    let cal = Calibration {
        dmin: DVec3::new(0.14, 0.098, 0.094),
        dmax: DVec3::new(0.017, 0.0052, 0.0029),
        input_floor: 1. / 65535.,
    };
    let truth = RollSettings {
        balance: Balance {
            warmth: -0.56,
            tint: -0.03,
        },
        ..RollSettings::new_grade()
    };
    let matrix = DMat3::from_cols(
        DVec3::new(1.1, 0.03, -0.07),
        DVec3::new(0.2, 0.8, -0.1),
        DVec3::new(-0.03, 0.08, 1.3),
    );
    let target = matrix.inverse() * DVec3::ONE;
    let initial = RollSettings {
        balance: Balance::default(),
        ..truth
    };
    let inverse = Inversion::new(cal, truth).unwrap();
    let mut points = Vec::new();
    for i in 0..90 {
        let y = 2_f64.powf(-7. + i as f64 / 89. * 6.8);
        let p = forward(cal, truth, target * y).unwrap();
        assert!(p.min_element() >= cal.input_floor);
        assert!(
            (matrix * inverse.apply(p).unwrap().rgb - DVec3::splat(y))
                .abs()
                .max_element()
                < 1e-11
        );
        points.push(p);
    }
    for p in [
        points.as_slice(),
        &points[..30],
        &points[30..60],
        &points[60..],
    ] {
        let b = crate::sampling::roll_neutral(cal, initial, p, matrix).unwrap();
        assert!((b.warmth - truth.balance.warmth).abs() < 1e-8);
        assert!((b.tint - truth.balance.tint).abs() < 1e-8);
    }
    let spans = (cal.dmin / cal.dmax).ln();
    let coords =
        crate::color::opponent_coordinates((spans / (spans.x * spans.y * spans.z).cbrt()).ln());
    let span_roll = RollSettings {
        balance: Balance {
            warmth: coords.x,
            tint: coords.y,
        },
        ..initial
    };
    let span_inv = Inversion::new(cal, span_roll).unwrap();
    for t in [0.05, 0.1, 0.2, 0.4, 0.6, 0.8, 1.] {
        let p = cal.dmin * (-spans * t).exp();
        let rgb = span_inv.apply(p).unwrap().rgb;
        assert!((rgb / rgb.y - DVec3::ONE).abs().max_element() < 1e-11);
    }
    let colour = DVec3::new(0.15, 0.09, 0.04);
    let wb = crate::color::wb_matrix(crate::color::neutral_wb(colour).unwrap()).unwrap();
    for level in [0.01, 0.1, 1., 2.] {
        let rgb = wb * colour * level;
        assert!((rgb / rgb.y - DVec3::ONE).abs().max_element() < 1e-12);
    }
    let y = vec![1., 2., 3., 4., 10., 20., 30., 40.];
    assert_eq!(band_indices(&y, 4, 0., 0.5, true), vec![0, 1, 4, 5]);
    assert_eq!(band_indices(&y, 4, 0., 0.5, false), vec![0, 1, 2, 3]);
}

#[test]
#[ignore = "private tonal gamma diagnosis; explicit photo-exposure report and fresh output"]
fn diagnose_gamma_tones() -> Result<(), Box<dyn std::error::Error>> {
    let report: Value = serde_json::from_slice(&fs::read(PathBuf::from(
        std::env::var_os("ANTITONE_ROLL_REPORT").ok_or("missing report")?,
    ))?)?;
    let output = PathBuf::from(std::env::var_os("ANTITONE_ROLL_OUTPUT").ok_or("missing output")?);
    let names: Vec<_> = std::env::var("ANTITONE_ROLL_FRAMES")?
        .split(',')
        .map(str::to_owned)
        .collect();
    let project: crate::project::Project =
        serde_json::from_value(report["baseline_project"].clone())?;
    let before = serde_json::to_value(&project)?;
    let roll: RollSettings = serde_json::from_value(report["roll"].clone())?;
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
    let reconstructed = crate::sampling::roll_neutral(cal, initial, &points, matrix)?;
    assert!((reconstructed.warmth - roll.balance.warmth).abs() < 1e-8);
    assert!((reconstructed.tint - roll.balance.tint).abs() < 1e-8);
    let inv = Inversion::new(cal, initial)?;
    let native = points
        .iter()
        .map(|&p| inv.apply(p).map(|v| v.rgb))
        .collect::<Result<Vec<_>, _>>()?;
    let y: Vec<_> = native
        .iter()
        .map(|p| crate::color::AP1_TO_XYZ.row(1).dot(matrix * *p))
        .collect();
    let mean_native = crate::sampling::mean(&native)?;
    let mean_ap1 = matrix * mean_native;
    let common_wb = crate::color::neutral_wb(mean_ap1)?;
    let mut variants = vec![("current", roll, Balance::default())];
    let mut fits = Vec::new();
    for (label, low, high, relative) in [
        ("relative-shadow", 0.01, 0.33, true),
        ("relative-mid", 0.33, 0.67, true),
        ("relative-high", 0.67, 0.99, true),
        ("pooled-shadow", 0.01, 0.33, false),
        ("pooled-mid", 0.33, 0.67, false),
        ("pooled-high", 0.67, 0.99, false),
    ] {
        let ids = band_indices(&y, count, low, high, relative);
        let selected: Vec<_> = ids.iter().map(|&i| points[i]).collect();
        let mut counts = vec![0; project.order.len()];
        for i in &ids {
            counts[*i / count] += 1;
        }
        match crate::sampling::roll_neutral(cal,initial,&selected,matrix) {
            Ok(balance)=>{
                println!("tone {label}: {balance:?}");
                fits.push(json!({"variant":label,"balance":balance,"count":selected.len(),"frame_counts":counts,"pre_fit_y_min_max":[ids.iter().map(|i|y[*i]).min_by(f64::total_cmp),ids.iter().map(|i|y[*i]).max_by(f64::total_cmp)]}));
                variants.push((label,RollSettings {balance,..roll},Balance::default()));
            },
            Err(error)=>fits.push(json!({"variant":label,"error":error.to_string(),"count":selected.len(),"frame_counts":counts})),
        }
    }
    let mut pixel_weights = Vec::new();
    for ys in y.chunks_exact(count) {
        let mut w: Vec<_> = ys.iter().map(|v| 1. / v.sqrt()).collect();
        let scale = count as f64 / w.iter().sum::<f64>();
        for v in &mut w {
            *v *= scale;
        }
        pixel_weights.extend(w);
    }
    let balance = fit(cal, initial, matrix, &points, &pixel_weights)?;
    variants.push((
        "pixel-root-Y",
        RollSettings { balance, ..roll },
        Balance::default(),
    ));
    // A straight normalized-density axis is a diagnostic assumption, not identified film gray.
    let spans = (cal.dmin / cal.dmax).ln();
    let relative = spans / (spans.x * spans.y * spans.z).cbrt();
    let coordinates = crate::color::opponent_coordinates(relative.ln());
    let span_roll = RollSettings {
        balance: Balance {
            warmth: coordinates.x,
            tint: coordinates.y,
        }
        .validate()?,
        ..roll
    };
    variants.push(("span-axis", span_roll, Balance::default()));
    let span_inv = Inversion::new(cal, span_roll)?;
    let span_pixels = points
        .iter()
        .map(|&p| span_inv.apply(p).map(|v| matrix * v.rgb))
        .collect::<Result<Vec<_>, _>>()?;
    let span_wb = crate::color::neutral_wb(crate::sampling::mean(&span_pixels)?)?;
    variants.push(("span-axis-WB", span_roll, span_wb));
    variants.push(("constant-WB-control", initial, common_wb));
    let mut audit = Vec::new();
    for (i, &id) in project.order.iter().enumerate() {
        let record = project.source(id)?;
        let start = i * count;
        let end = start + count;
        let bands = [(0.01, 0.33), (0.33, 0.67), (0.67, 0.99)]
            .map(|(low, high)| band_indices(&y[start..end], count, low, high, false));
        let mut measures = Vec::new();
        for (name, r, wb) in &variants {
            let inv = Inversion::new(cal, *r)?;
            let post = crate::color::wb_matrix(*wb)? * matrix;
            let levels=bands.iter().map(|ids| {
                let mut sum=DVec3::ZERO;for &j in ids {sum+=post*inv.apply(points[start+j])?.rgb;}
                let mean=sum/ids.len() as f64;let luminance=crate::color::AP1_TO_XYZ.row(1).dot(mean);
                Ok(json!({"ap1_mean":mean.to_array(),"Y":luminance,"rgb_over_y":if luminance.abs()>1e-8 {Some((mean/luminance).to_array())} else {None}}))
            }).collect::<Result<Vec<_>,Error>>()?;
            measures.push(json!({"variant":name,"shadow_mid_high":levels}));
        }
        audit.push(json!({"file":record.path.file_stem().ok_or("no stem")?.to_string_lossy(),"variants":measures}));
    }
    // This is a model-defined constant-chromaticity axis, not measured neutral film.
    let base_y = crate::color::AP1_TO_XYZ.row(1).dot(mean_ap1);
    let mut ramp = Vec::new();
    for level in [0.005, 0.01, 0.02, 0.05, 0.1, 0.18, 0.3, 0.5, 0.8, 1., 2.] {
        let scan = forward(cal, initial, mean_native * (level / base_y))?;
        if scan.min_element() < cal.input_floor {
            ramp.push(json!({"input_y":level,"outside_scan_floor":true}));
            continue;
        }
        let values=variants.iter().map(|(name,r,wb)| {
            let rgb=crate::color::wb_matrix(*wb)?*matrix*Inversion::new(cal,*r)?.apply(scan)?.rgb;
            let y=crate::color::AP1_TO_XYZ.row(1).dot(rgb);
            Ok(json!({"variant":name,"rgb":rgb.to_array(),"Y":y,"rgb_over_y":(rgb/y).to_array()}))
        }).collect::<Result<Vec<_>,Error>>()?;
        ramp.push(json!({"input_y":level,"values":values}));
    }
    let span_axis: Vec<_> = [0.05, 0.1, 0.2, 0.4, 0.6, 0.8, 1.]
        .into_iter()
        .map(|t| {
            let scan = cal.dmin * (-spans * t).exp();
            let values = variants
                .iter()
                .map(|(name, r, wb)| {
                    let rgb = crate::color::wb_matrix(*wb)?
                        * matrix
                        * Inversion::new(cal, *r)?.apply(scan)?.rgb;
                    let y = crate::color::AP1_TO_XYZ.row(1).dot(rgb);
                    Ok(json!({"variant":name,"rgb_over_y":(rgb/y).to_array()}))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Ok(json!({"normalized_density":t,"values":values}))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    fs::create_dir(&output)?;
    let renderer = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?;
    let wb = project
        .development
        .as_ref()
        .ok_or("no development")?
        .fixed_wb;
    let mut rendered = Vec::new();
    for &id in &project.order {
        let record = project.source(id)?;
        let name = record.path.file_stem().ok_or("no stem")?.to_string_lossy();
        if !names.iter().any(|n| n == name.as_ref()) {
            continue;
        }
        let f = report["frames"]
            .as_array()
            .ok_or("no exposure frames")?
            .iter()
            .find(|f| f["file"] == name.as_ref())
            .ok_or("no exposure frame")?;
        let p = f["variants"]
            .as_array()
            .ok_or("no exposures")?
            .iter()
            .find(|v| v["variant"] == "photo-robust-blend99")
            .ok_or("no robust exposure")?;
        let photo: PhotoSettings = serde_json::from_value(p["photo"].clone())?;
        assert_eq!(
            photo.wb,
            Balance::default(),
            "operator control expects zero original Photo WB"
        );
        let mut scan = Scan::open(&record.path, Some(wb))?;
        document::verify_scan(record, &scan)?;
        if let Some(field) = project.active_flat_field() {
            scan.set_flat_field(Some(field.map.clone()))?;
        }
        let (w, h) = scan.preview_dimensions(1024)?;
        let grid = crate::geometry::SourceRaster::new(scan.area, [w, h])?;
        let mut pixels = Vec::new();
        for yy in 0..h {
            for xx in 0..w {
                let [sx, sy] = grid.source_pixel([xx, yy])?;
                pixels.push(scan.pixel(sx, sy)?);
            }
        }
        let mut values = Vec::new();
        for (label, r, common) in &variants {
            let candidate = PhotoSettings {
                wb: *common,
                ..photo
            };
            assert_eq!(candidate.exposure, photo.exposure);
            assert_eq!(
                RollSettings {
                    balance: roll.balance,
                    ..*r
                },
                roll
            );
            let pipe = Pipeline::new(cal, *r, candidate, matrix)?;
            let working = pixels
                .iter()
                .map(|&p| pipe.apply(p).map(|v| v.rgb))
                .collect::<Result<Vec<_>, _>>()?;
            write_preview(
                &output.join(format!("{name}-{label}.tif")),
                [w, h],
                &OutputTarget::Srgb.encode_u16(&renderer.render(&working)?)?,
            )?;
            values.push(json!({"variant":label,"roll":r,"photo":candidate,"preview_y_percentiles_1_5_50_95_99":percentiles(working.iter().map(|p|crate::color::AP1_TO_XYZ.row(1).dot(*p)))}));
        }
        rendered.push(json!({"file":name,"variants":values}));
    }
    assert_eq!(rendered.len(), names.len());
    assert_eq!(serde_json::to_value(&project)?, before);
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"baseline_project":project,"fixed_roll":roll,"common_WB_control":common_wb,"samples_per_frame":count,"fits":fits,"audit":audit,"mean_colour_model_axis":ramp,"straight_normalized_density_model_axis":span_axis,"span_axis_roll":span_roll,"span_axis_WB":span_wb,"rendered":rendered,"contract":"Diagnostic copies only. Same current fit reconstructed; fixed endpoints/model/exposures. Six tone fits use original pixels in frozen unbalanced-Y ranks: per-frame bands preserve equal frame counts, pooled bands may change scene influence (counts reported). Bands are 1–33/33–67/67–99 percentiles, not identified neutral materials. Fit failures reported without fallback. Pixel-root-Y uses frozen inverse-square-root Y weights with equal frame total. Span-axis gamma ratios come from log(Dmin/Dmax), preserving global gamma and making a straight normalized-density native axis achromatic for a normalized camera basis; this is a diagnostic model axis, NOT measured neutral film. Span-axis-WB adds existing Bradford mean correction without fitting channel curves to scene mean. Existing Bradford WB on zero-balanced inversion is an operator control using the SAME unbalanced whole-roll mean, not a selected processing replacement. Model-defined mean-colour ramp is not a camera/film gray reference. All source/pixel data, ACES and mathematical gates unchanged. 1024px nearest-centre previews, original project unchanged."}),
        )?,
    )?;
    Ok(())
}
