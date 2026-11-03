use super::*;
use serde::Deserialize;
use std::sync::atomic::Ordering;

#[derive(Deserialize)]
struct Fixtures {
    inputs: Vec<DVec3>,
    encoded: Vec<DVec3>,
    reference_rgb: Vec<DVec3>,
    log_h: Vec<f64>,
    density: Vec<f64>,
}
fn fixtures() -> Fixtures {
    serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/papers/ilford-multigrade-rc-deluxe-filter-2.json"
    ))
    .unwrap()
}
fn model() -> Model {
    Model::new(Paper::default())
}

#[test]
fn scalar_exposure_development_and_readout_match_executed_upstream() {
    let f = fixtures();
    let m = model();
    let count = f.inputs.len();
    assert_eq!(count, f.encoded.len());
    assert_eq!(count, f.reference_rgb.len());
    assert_eq!(count, f.log_h.len());
    assert_eq!(count, f.density.len());
    assert!(f.inputs.iter().any(|rgb| rgb.min_element() < 0.));
    assert!(f.inputs.iter().any(|rgb| rgb.max_element() > 1.));
    assert!(
        f.inputs
            .iter()
            .any(|rgb| rgb.max_element() - rgb.min_element() > 1.)
    );
    let mut maximum: f64 = 0.;
    for i in 0..count {
        assert!(
            (m.exposure(f.inputs[i]) - f.log_h[i]).abs() < 2e-12,
            "exposure case {i}"
        );
        assert!(
            (m.density(f.log_h[i]) - f.density[i]).abs() < 2e-12,
            "density case {i}"
        );
        maximum = maximum.max(
            (m.encoded(f.inputs[i]).unwrap() - f.encoded[i])
                .abs()
                .max_element(),
        );
    }
    println!("{count} B&W source probes, max encoded readout error {maximum}");
    // The generated readout's dense source comparison is below 1e-6 encoded;
    // this also admits table interpolation plus f64/f32 reference roundoff.
    assert!(maximum < 2e-6);
}

#[test]
fn luminance_precedes_the_nonlinear_curve_and_density_endpoints_extend_constantly() {
    let m = model();
    let gray = DVec3::splat(0.18);
    let weights = color::AP1_TO_XYZ.row(1);
    let equal_y = gray + DVec3::new(0.2, -0.2 * weights.x / weights.y, 0.);
    assert!(
        (m.encoded(gray).unwrap() - m.encoded(equal_y).unwrap())
            .abs()
            .max_element()
            < 1e-12
    );
    // A chromatic no-op or channel-wise curve cannot satisfy both this invariant
    // and the contrasting luminance response.
    let low = m.encoded(DVec3::splat(0.01)).unwrap();
    let high = m.encoded(DVec3::splat(1.)).unwrap();
    assert!((high - low).min_element() > 0.1);
    let p = m.paper;
    assert_eq!(m.density(p.exposure.min - 100.), p.exposure.density[0]);
    assert_eq!(
        m.density(p.exposure.max + 100.),
        *p.exposure.density.last().unwrap()
    );
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(m.encoded(DVec3::new(0.18, bad, 0.18)).is_err());
    }
}

#[test]
fn paper_only_identity_and_baking_preserve_numeric_assets_without_catalogue_or_cache() {
    let m = model();
    let paper = Paper::default();
    assert_eq!(paper.id(), "ilford_multigrade_rc_deluxe_filter_2");
    assert_eq!(Paper::from_origin(&paper.origin()), Some(paper));
    let mut invalid_pair = paper.origin();
    let LookOrigin::Photochemical { negative, .. } = &mut invalid_pair else {
        unreachable!()
    };
    *negative = Some("kodak_portra_400".into());
    assert_eq!(Paper::from_origin(&invalid_pair), None);
    let cancel = AtomicBool::new(false);
    let asset = m.bake(3, &cancel).unwrap();
    let encoded = serde_json::to_vec(&asset).unwrap();
    let restored: LookAsset = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(restored, asset);
    assert!(matches!(
        restored.origin(),
        LookOrigin::Photochemical { negative: None, .. }
    ));
    assert!(m.bake(1, &cancel).is_err());
    assert!(m.bake(super::super::MAX_GRID_SIZE + 1, &cancel).is_err());
    cancel.store(true, Ordering::Relaxed);
    assert!(m.bake(3, &cancel).is_err());
}

mod catalogue;
#[cfg(feature = "gpu")]
mod gpu;
#[cfg(all(feature = "gpu", feature = "raw"))]
mod photo;
