use super::*;
use crate::looks::frontier::{Model, Selection};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    inputs: Vec<DVec3>,
    stocks: Vec<Stock>,
}
#[derive(Deserialize)]
struct Stock {
    id: String,
    density: Vec<DVec3>,
    #[serde(rename = "OD")]
    od: Vec<DVec3>,
    outputs: BTreeMap<String, Output>,
}
#[derive(Deserialize)]
struct Output {
    codes: Vec<DVec3>,
    #[serde(rename = "AP1")]
    ap1: Vec<DVec3>,
    lut_identity: String,
}

#[test]
fn fixed_exposure_stock_readout_and_authoring_match_independent_accepted_controls() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/frontier/negative-controls.json"
    ))
    .unwrap();
    assert_eq!(fixture.stocks.len(), spektrafilm::Negative::all().len());
    assert_eq!(fixture.inputs.len(), 49);
    let cancel = AtomicBool::new(false);
    for stock in fixture.stocks {
        let negative = spektrafilm::Negative::from_id(&stock.id).unwrap();
        let entry = Entry::new(negative, Variant::A, &cancel).unwrap();
        for (index, &input) in fixture.inputs.iter().enumerate() {
            assert!(
                (entry.scanner.formation.density(input) - stock.density[index])
                    .abs()
                    .max_element()
                    < 2e-12,
                "{}/density/{index}",
                stock.id
            );
            assert!(
                (entry.scanner.optical_density(input) - stock.od[index])
                    .abs()
                    .max_element()
                    < 2e-12,
                "{}/OD/{index}",
                stock.id
            );
        }
        for (variant, key) in [(Variant::A, "A"), (Variant::B, "B")] {
            let selection = Selection {
                negative: Some(negative),
                variant,
            };
            let model = Model::new(selection, &cancel).unwrap();
            let entry = Entry::new(negative, variant, &cancel).unwrap();
            let expected = &stock.outputs[key];
            for (index, &input) in fixture.inputs.iter().enumerate() {
                assert!(
                    (entry.coordinates(input).unwrap() - expected.codes[index])
                        .abs()
                        .max_element()
                        < 2e-10,
                    "{}/{key}/codes/{index}",
                    stock.id
                );
                assert!(
                    (model.ap1(input).unwrap() - expected.ap1[index])
                        .abs()
                        .max_element()
                        < 2e-10,
                    "{}/{key}/AP1/{index}",
                    stock.id
                );
            }
            let baked = selection.prepare(&cancel).unwrap();
            assert_eq!(
                baked.id().to_string(),
                expected.lut_identity,
                "{}/{key}/65LUT",
                stock.id
            );
            assert!(model.ap1(DVec3::splat(f64::NAN)).is_err());
            assert!(model.ap1(DVec3::splat(f64::INFINITY)).is_err());
        }
        assert!(
            Model::new(
                Selection {
                    negative: Some(negative),
                    variant: Variant::A
                },
                &AtomicBool::new(true)
            )
            .err()
            .unwrap()
            .to_string()
            .contains("cancelled")
        );
    }
}

#[test]
fn stock_readout_has_only_diagonal_neutral_slopes_and_keeps_native_grey() {
    let stock = spektrafilm::Negative::from_id("kodak_portra_400").unwrap();
    for variant in Variant::all() {
        let entry = Entry::new(stock, variant, &AtomicBool::new(false)).unwrap();
        for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
            let actual =
                entry.raw_codes(entry.grey + axis / entry.balance) - entry.readout.grey_codes;
            let expected =
                entry.definition.codes_per_od * (entry.neutral * axis) * entry.readout.gains;
            assert!((actual - expected).abs().max_element() < 1e-12);
        }
    }
    for stock in spektrafilm::Negative::all() {
        for variant in Variant::all() {
            let model = Model::new(
                Selection {
                    negative: Some(stock),
                    variant,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(
                (model.ap1(DVec3::splat(0.18)).unwrap() - DVec3::splat(0.18))
                    .abs()
                    .max_element()
                    < 1e-12
            );
        }
    }
}

#[test]
fn stock_entries_share_readout_characterization_and_native_anchors() {
    let cancel = AtomicBool::new(false);
    let portra = spektrafilm::Negative::from_id("kodak_portra_400").unwrap();
    for variant in Variant::all() {
        let reference = Entry::new(portra, variant, &cancel).unwrap();
        let positive = super::super::positive::Entry::new(variant).unwrap();
        assert!(
            (positive.coordinates(DVec3::splat(0.18)).unwrap() - reference.readout.grey_codes)
                .abs()
                .max_element()
                < 1e-12
        );
        for stock in spektrafilm::Negative::all() {
            let selected = Entry::new(stock, variant, &cancel).unwrap();
            assert!(std::ptr::eq(reference.readout, selected.readout));
            assert_eq!(reference.readout.grey_codes, selected.readout.grey_codes);
            for density in [DVec3::ZERO, DVec3::X, DVec3::new(0.3, -0.2, 0.7)] {
                let a = reference.raw_codes(reference.grey + density / reference.balance);
                let b = selected.raw_codes(selected.grey + density / selected.balance);
                assert!(
                    (a - b).abs().max_element() < 1e-10,
                    "{} {variant:?}",
                    stock.id()
                );
            }
        }
    }
}

#[test]
fn bank_readouts_are_affine_with_the_recorded_neutral_slope_relationship() {
    let stock = spektrafilm::Negative::from_id("kodak_portra_400").unwrap();
    let cancel = AtomicBool::new(false);
    let a = Entry::new(stock, Variant::A, &cancel).unwrap();
    let b = Entry::new(stock, Variant::B, &cancel).unwrap();
    let sa = a.neutral;
    let sb = b.neutral;
    let x = DVec3::new(0.1, 0.5, 0.9);
    let y = DVec3::new(0.8, 0.2, 1.2);
    let t = 0.37;
    for entry in [&a, &b] {
        assert_eq!(entry.raw_codes(entry.grey), entry.readout.grey_codes);
        let interpolated = entry.raw_codes(x * (1. - t) + y * t);
        let expected = entry.raw_codes(x) * (1. - t) + entry.raw_codes(y) * t;
        assert!((interpolated - expected).abs().max_element() < 1e-12);
    }
    for density in [DVec3::ZERO, x, y, DVec3::splat(4.)] {
        let ca = a.raw_codes(density) - a.readout.grey_codes;
        let cb = b.raw_codes(density) - b.readout.grey_codes;
        assert!((cb - ca * (sb / sa)).abs().max_element() < 2e-12);
    }
}

#[test]
fn uniform_scanner_od_scaling_retains_centred_code_span_strength() {
    for stock in spektrafilm::Negative::all() {
        for variant in Variant::all() {
            let entry = Entry::new(stock, variant, &AtomicBool::new(false)).unwrap();
            for factor in [0.5, 2., 3.] {
                let mut scaled = Entry::new(stock, variant, &AtomicBool::new(false)).unwrap();
                // Hypothetical scanner OD scaling, not a change to negative exposure or chemistry.
                scaled.grey *= factor;
                scaled.balance = DVec3::splat(scaled.grey.element_sum() / 3.) / scaled.grey;
                assert!((scaled.balance - entry.balance).abs().max_element() < 1e-14);
                for density in [
                    DVec3::ZERO,
                    entry.grey,
                    DVec3::new(0.2, 1.4, 0.7),
                    DVec3::splat(3.),
                ] {
                    let expected = factor * (entry.raw_codes(density) - entry.readout.grey_codes);
                    let actual = scaled.raw_codes(factor * density) - scaled.readout.grey_codes;
                    assert!(
                        (actual - expected).abs().max_element() < 1e-10,
                        "{} {variant:?} {factor}",
                        stock.id()
                    );
                }
            }
        }
    }
}

#[cfg(feature = "ocio-reference")]
#[test]
fn catalogue_black_matches_approved_native_controls_without_an_output_override() {
    use crate::rendering::{CpuRenderer, OutputTarget};
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/frontier/negative-controls.json"
    ))
    .unwrap();
    let zero = fixture
        .inputs
        .iter()
        .position(|&v| v == DVec3::ZERO)
        .unwrap();
    let mut blacks = Vec::new();
    for stock in spektrafilm::Negative::all() {
        for variant in Variant::all() {
            let model = Model::new(
                Selection {
                    negative: Some(stock),
                    variant,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            let control = fixture.stocks.iter().find(|s| s.id == stock.id()).unwrap();
            let key = format!("{variant:?}");
            blacks.push((
                stock.id(),
                variant,
                model.ap1(DVec3::ZERO).unwrap(),
                control.outputs[&key].ap1[zero],
            ));
        }
    }
    let carriers: Vec<_> = blacks.iter().map(|(_, _, black, _)| *black).collect();
    let controls: Vec<_> = blacks.iter().map(|(_, _, _, black)| *black).collect();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let renderer = CpuRenderer::new(target).unwrap();
        let rendered = renderer.render(&carriers).unwrap();
        let encoded = target.encode_u16(&rendered).unwrap();
        let expected = target
            .encode_u16(&renderer.render(&controls).unwrap())
            .unwrap();
        for (((stock, variant, _, _), codes), reference) in blacks.iter().zip(encoded).zip(expected)
        {
            assert_eq!(codes, reference, "{stock}/{variant:?}/{target:?}");
            match variant {
                Variant::A => {
                    if *stock == "kodak_portra_400" {
                        assert!(
                            codes[0] > codes[1] && codes[0] > codes[2],
                            "{target:?}/warm floor"
                        );
                    }
                }
                Variant::B => assert!(
                    codes.iter().all(|v| *v > 0),
                    "{stock}/{variant:?}/{target:?}"
                ),
            }
        }
    }
}

#[test]
fn scanner_balances_base_flux_and_only_grey_chroma_for_stocks() {
    let cancel = AtomicBool::new(false);
    for stock in spektrafilm::Negative::all() {
        let entry = Entry::new(stock, Variant::A, &cancel).unwrap();
        assert_eq!(entry.scanner.attenuation(entry.scanner.base), DVec3::ZERO);
        let mass = entry
            .scanner
            .bands
            .iter()
            .fold(DVec3::ZERO, |sum, sample| sum + sample.weight);
        assert!((mass - DVec3::ONE).abs().max_element() < 2e-15);
        let gray = entry.scanner.optical_density(DVec3::splat(0.18));
        let balanced = gray * entry.balance;
        assert!(
            (balanced - DVec3::splat(gray.element_sum() / 3.))
                .abs()
                .max_element()
                < 1e-14
        );
        for scale in [1e-9, 1e-5, 0.01, 1.] {
            let delta = DVec3::new(scale, scale * 0.7, scale * 0.3);
            let flux = entry.scanner.bands.iter().fold(DVec3::ZERO, |sum, sample| {
                sum + sample.weight * 10_f64.powf(-sample.dyes.dot(delta))
            });
            let expected = -flux.map(f64::ln);
            assert!(
                (entry.scanner.attenuation(entry.scanner.base + delta) - expected)
                    .abs()
                    .max_element()
                    < 2e-13
            );
        }
    }
}
