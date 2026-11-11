use super::*;
use crate::looks::spektrafilm::{Model, Paper, Selection};
use crate::photographic;
use std::sync::atomic::AtomicBool;

fn model() -> Model {
    Model::new(
        Selection {
            negative: None,
            paper: Paper::from_id("kodak_supra_endura").unwrap(),
        },
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn neutral_tone_reference_retains_signed_tangent_and_extreme_inputs_remain_finite() {
    let m = model();
    let mapping = &data::catalog().paper_mapping;
    let reference = |x| reference_exposure(m.paper, x, mapping);
    let zero = reference(DVec3::ZERO);
    let negative = reference(DVec3::splat(-0.02));
    assert!(negative.min_element() > zero.max_element());
    let break_point = 0.0078125;
    for x in [-0.054, -0.02, 0., 0.003, break_point] {
        let expected_difference = -mapping.exposure_compression * (x - break_point)
            / (break_point * std::f64::consts::LN_10);
        let observed = reference(DVec3::splat(x)) - reference(DVec3::splat(break_point));
        assert!(
            (observed - DVec3::splat(expected_difference))
                .abs()
                .max_element()
                < 1e-12
        );
    }
    for x in [
        DVec3::new(-0.0212899204, 0.0399119705, -0.0189858880),
        DVec3::new(-100., 1., 1024.),
        DVec3::new(1e100, -1e100, 0.),
        DVec3::new(1e-100, -2e-100, 3e-100),
    ] {
        assert!(reference(x).is_finite());
        assert!(m.paper_exposure(x).unwrap().is_finite());
    }
}

#[test]
fn complete_neutral_trajectory_is_unchanged_including_allocation_toe_and_extrapolation() {
    let m = model();
    let p = &data::catalog().paper_mapping;
    let mut values: Vec<_> = (0..=4096)
        .map(|i| photographic::decode(-0.5 + 1.6 * i as f64 / 4096.))
        .collect();
    values.extend([-100., -1., 0., 0.0078125, 0.18, 1024., 1e6]);
    for x in values {
        let expected = m.paper.positive_gray_log_h
            - DVec3::splat(
                p.exposure_compression
                    * 17.52
                    * (photographic::encode(x) - photographic::encode(p.gray_ap1))
                    * std::f64::consts::LOG10_2,
            );
        assert!(
            (m.paper_exposure(DVec3::splat(x)).unwrap() - expected)
                .abs()
                .max_element()
                < 1e-12
        );
    }
}

#[test]
fn colour_changes_do_not_promote_dark_input_to_bright_paper_and_are_not_a_no_op() {
    let m = model();
    let c = m.paper.positive_driver.as_ref().unwrap();
    let direction = DVec3::new(0.7, -0.4, 1.2);
    let opponent = direction - DVec3::splat(c.input_luminance.dot(direction));
    for brightness in [-0.1, 0., 0.002, 0.02, 0.18, 0.8, 4., 1024.] {
        let neutral = DVec3::splat(brightness);
        let reference = c.luminance(m.paper, m.paper_exposure(neutral).unwrap());
        for amount in [-2., -0.05, 0., 0.03, 2.] {
            let input = neutral + opponent * amount;
            let h = m.paper_exposure(input).unwrap();
            assert!(h.is_finite());
            assert!((c.luminance(m.paper, h) - reference).abs() < 2e-12);
        }
    }
    let neutral = m.encoded(DVec3::splat(0.18)).unwrap();
    let coloured = m.encoded(DVec3::splat(0.18) + opponent * 0.03).unwrap();
    assert!((coloured - neutral).abs().max_element() > 0.005);
    // Supplied original-pixel regression: the old global log matrix made this
    // finite signed, low-luminance input a bright green. Preserve the tone invariant,
    // not a photograph-specific RGB colour or a "noise-free sky" expectation.
    let x = DVec3::new(
        -0.021289920434355736,
        0.03991197049617767,
        -0.018985887989401817,
    );
    let gray = DVec3::splat(c.input_luminance.dot(x));
    assert!(
        (c.luminance(m.paper, m.paper_exposure(x).unwrap())
            - c.luminance(m.paper, m.paper_exposure(gray).unwrap()))
        .abs()
            < 2e-12
    );
}

#[test]
fn spectral_photometry_uses_original_dye_base_weights_and_has_a_matching_common_slope() {
    let m = model();
    let c = m.paper.positive_driver.as_ref().unwrap();
    for h in [
        DVec3::new(-0.4, 0.2, 0.8),
        m.paper.positive_gray_log_h,
        DVec3::new(1.1, 0.9, 1.4),
    ] {
        let density = develop_print_morph(m.paper, h);
        let mut sum = 0.;
        for i in 0..c.photometry.weights.len() {
            let d = c.photometry.dyes[i];
            sum += c.photometry.weights[i]
                * 10_f64.powf(-(d.x * density.x + d.y * density.y + d.z * density.z));
        }
        let (y, derivative) = c.luminance_and_slope(m.paper, h);
        assert!((y - sum).abs() < 1e-14);
        assert!(derivative < 0.);
        for step in [1e-5, 3e-6] {
            let observed = (c.luminance(m.paper, h + DVec3::splat(step))
                - c.luminance(m.paper, h - DVec3::splat(step)))
                / (2. * step);
            assert!((derivative - observed).abs() < 1e-7);
        }
    }
}

#[test]
fn nonpositive_input_y_keeps_the_original_gray_path_and_former_fold_queries_solve() {
    let m = model();
    let c = m.paper.positive_driver.as_ref().unwrap();
    for input in [DVec3::new(0.01, -0.05, 0.02), DVec3::new(-1., -0.2, 0.5)] {
        let y = c.input_luminance.dot(input);
        assert!(y < 0.);
        assert_eq!(
            m.paper_exposure(input).unwrap(),
            reference_exposure(m.paper, DVec3::splat(y), &data::catalog().paper_mapping)
        );
    }
    for input in [
        DVec3::new(32., 17.6, 0.2),
        DVec3::new(22.255227981125067, 12.411862377076536, -0.05435413775647024),
    ] {
        let h = m.paper_exposure(input).unwrap();
        let y = c.input_luminance.dot(input);
        let neutral = reference_exposure(m.paper, DVec3::splat(y), &data::catalog().paper_mapping);
        assert!(h.is_finite());
        assert!((c.luminance(m.paper, h) - c.luminance(m.paper, neutral)).abs() < 2e-12);
    }
}

#[test]
#[ignore = "Set ANTITONE_SUPRA_POINTS (f32 AP1) and ANTITONE_SUPRA_EXPOSURES (f64 source logH)"]
fn captured_sites_match_source_exposure_differences_density_and_original_y() {
    let inputs =
        std::fs::read(std::env::var_os("ANTITONE_SUPRA_POINTS").expect("captured AP1 points"))
            .unwrap();
    let expected = std::fs::read(
        std::env::var_os("ANTITONE_SUPRA_EXPOSURES").expect("independent source exposures"),
    )
    .unwrap();
    assert!(!inputs.is_empty() && inputs.len().is_multiple_of(12));
    assert!(expected.len().is_multiple_of(24));
    assert_eq!(inputs.len() / 12, expected.len() / 24);
    let m = model();
    let driver = m.paper.positive_driver.as_ref().unwrap();
    let mut raw: f64 = 0.;
    let mut pairwise: f64 = 0.;
    let mut density: f64 = 0.;
    let mut luminance: f64 = 0.;
    for (input, source) in inputs
        .as_chunks::<12>()
        .0
        .iter()
        .zip(expected.as_chunks::<24>().0)
    {
        let input = DVec3::from_array(std::array::from_fn(|c| {
            f32::from_le_bytes(input[4 * c..4 * c + 4].try_into().unwrap()) as f64
        }));
        let source = DVec3::from_array(std::array::from_fn(|c| {
            f64::from_le_bytes(source[8 * c..8 * c + 8].try_into().unwrap())
        }));
        assert!(input.is_finite() && source.is_finite());
        let actual = m.paper_exposure(input).unwrap();
        let difference = actual - source;
        raw = raw.max(difference.abs().max_element());
        pairwise = pairwise.max(
            (difference - DVec3::splat(difference.x))
                .abs()
                .max_element(),
        );
        density = density.max(
            (develop_print_morph(m.paper, actual) - develop_print_morph(m.paper, source))
                .abs()
                .max_element(),
        );
        let neutral = reference_exposure(
            m.paper,
            DVec3::splat(driver.input_luminance.dot(input)),
            &data::catalog().paper_mapping,
        );
        luminance = luminance
            .max((driver.luminance(m.paper, actual) - driver.luminance(m.paper, neutral)).abs());
    }
    println!(
        "{} source poses: raw logH {raw}, pairwise {pairwise}, density {density}, physical Y {luminance}",
        inputs.len() / 12
    );
    // On flat characteristic segments a common exposure coordinate is not unique.
    // Compare the invariant colour differences and developed material, not that gauge.
    assert!(pairwise < 1e-9);
    assert!(density < 1e-9);
    assert!(luminance < 2e-12);
}

#[test]
fn malformed_reference_and_photometry_data_are_rejected() {
    let p = model();
    let catalog: serde_json::Value =
        serde_json::from_slice(&data::decompress("catalog.json.z").unwrap()).unwrap();
    let value = catalog["paper"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == p.paper.id)
        .unwrap()["positive_driver"]
        .clone();
    let bytes = serde_json::to_vec(&value).unwrap();
    let make = || serde_json::from_slice::<Driver>(&bytes).unwrap();
    make().validate().unwrap();
    let mut obsolete: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    obsolete["opponent_matrix"] = serde_json::json!([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
    assert!(serde_json::from_value::<Driver>(obsolete).is_err());
    let mut c = make();
    c.input_luminance.x = -0.1;
    assert!(c.validate().is_err());
    let mut c = make();
    c.photometry.weights[0] = -0.1;
    assert!(c.validate().is_err());
    let mut c = make();
    c.photometry.dyes.pop();
    assert!(c.validate().is_err());
    let mut c = make();
    c.photometry.weights.fill(0.);
    assert!(c.validate().is_err());
}
