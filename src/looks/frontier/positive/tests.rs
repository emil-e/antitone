use super::*;

#[test]
fn compression_matches_independent_chart_drives_and_keeps_the_neutral_axis() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/frontier/positive-controls.json"
    ))
    .unwrap();
    let c = Definition::get().unwrap();
    let chart: Vec<DVec3> = serde_json::from_value(fixture["normal_inputs"].clone()).unwrap();
    assert_eq!(chart.len(), 144);
    let mixed_controls: Vec<DVec3> =
        serde_json::from_value(fixture["normal_mixed_drives"].clone()).unwrap();
    let compressed_controls: Vec<DVec3> =
        serde_json::from_value(fixture["normal_compressed_drives"].clone()).unwrap();
    let active: Vec<bool> =
        serde_json::from_value(fixture["normal_compression_active"].clone()).unwrap();
    assert_eq!(chart.len(), mixed_controls.len());
    assert_eq!(chart.len(), compressed_controls.len());
    assert_eq!(chart.len(), active.len());
    for (((x, expected_mixed), expected_compressed), active) in chart
        .into_iter()
        .zip(mixed_controls)
        .zip(compressed_controls)
        .zip(active)
    {
        let mixed = AP1_TO_LINEAR_REC709 * x;
        let compressed = c.compression.apply(mixed);
        assert!((mixed - expected_mixed).abs().max_element() < 1e-12);
        assert!((compressed - expected_compressed).abs().max_element() < 1e-12);
        if active {
            assert_ne!(compressed, mixed);
        } else {
            assert_eq!(compressed, mixed);
        }
    }
    for n in [-1000., -1., 0., 0.18, 356.] {
        let x = DVec3::splat(n);
        assert_eq!(c.compression.apply(x), x);
    }
}

#[test]
fn positive_basis_is_bradford_adapted_linear_rec709() {
    let primary = |x: f64, y: f64| DVec3::new(x / y, 1., (1. - x - y) / y);
    let primaries = DMat3::from_cols(
        primary(0.64, 0.33),
        primary(0.30, 0.60),
        primary(0.15, 0.06),
    );
    let white = primary(0.3127, 0.3290);
    let rec709_to_xyz = primaries * DMat3::from_diagonal(primaries.inverse() * white);
    let ap1_to_xyz = crate::color::AP1_TO_XYZ;
    let bradford = crate::color::BRADFORD;
    let adaptation = bradford.inverse()
        * DMat3::from_diagonal((bradford * white) / (bradford * ap1_to_xyz * DVec3::ONE))
        * bradford;
    let expected = rec709_to_xyz.inverse() * adaptation * ap1_to_xyz;
    for (actual, expected) in AP1_TO_LINEAR_REC709
        .to_cols_array()
        .into_iter()
        .zip(expected.to_cols_array())
    {
        assert!((actual - expected).abs() < 1e-12);
    }
}

#[test]
fn extreme_compression_is_radial_scale_equivariant_and_smooth_at_its_knee() {
    let c = Definition::get().unwrap().compression;
    for x in [
        DVec3::new(1., 0., 0.3),
        DVec3::new(1., -0.2, 0.4),
        DVec3::new(0.01, -1., -0.5),
    ] {
        let mapped = c.apply(x);
        assert!(mapped.min_element() > 0.);
        assert_eq!(mapped.max_element(), x.max_element());
        let a = x - DVec3::splat(x.max_element());
        let b = mapped - DVec3::splat(x.max_element());
        assert!(a.cross(b).length() < 1e-12);
        for scale in [1e-4, 1., 1e4] {
            assert!(
                (c.apply(scale * x) - scale * mapped).abs().max_element()
                    < 1e-10 * (1. + scale * mapped.abs().max_element())
            );
        }
    }
    let h = 1e-6;
    let distance = |r| 1. - c.apply(DVec3::new(1., 1. - r, 0.5)).y;
    assert!(((distance(c.knee + h) - distance(c.knee - h)) / (2. * h) - 1.).abs() < 1e-6);
    assert!((distance(c.knee) - c.knee).abs() < 1e-14);
}

#[test]
fn neutral_readout_matches_independently_sampled_bank_controls() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/frontier/positive-controls.json"
    ))
    .unwrap();
    let inputs: Vec<DVec3> = serde_json::from_value(fixture["neutral_inputs"].clone()).unwrap();
    assert_eq!(inputs.len(), 193);
    for (variant, bank) in [(Variant::A, "A"), (Variant::B, "B")] {
        let entry = Entry::new(variant).unwrap();
        let expected: Vec<DVec3> =
            serde_json::from_value(fixture["outputs"][bank]["neutral_codes"].clone()).unwrap();
        assert_eq!(inputs.len(), expected.len());
        for (&input, expected) in inputs.iter().zip(expected) {
            assert!(
                (entry.coordinates(input).unwrap() - expected)
                    .abs()
                    .max_element()
                    < 1e-10
            );
        }
    }
}

#[test]
fn fixed_basis_keeps_the_separately_calibrated_neutral_readout() {
    for variant in [Variant::A, Variant::B] {
        let entry = Entry::new(variant).unwrap();
        let c = entry.definition;
        for step in -64..=64 {
            let x = 0.18 * 2_f64.powf(f64::from(step) / 8.);
            let raw = entry.readout.grey_codes
                + c.codes_per_log
                    * entry.readout.gains
                    * *c.neutral_rows.get(variant)
                    * (characteristic::reference().coordinate(x) - entry.grey_coordinate);
            let expected = raw.map(|q| {
                if q < 0. {
                    0.
                } else if q > 1015. {
                    1023. - 64. / (q - 1007.)
                } else {
                    q
                }
            });
            assert!(
                (entry.coordinates(DVec3::splat(x)).unwrap() - expected)
                    .abs()
                    .max_element()
                    < 1e-12
            );
        }
    }
}
