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
                (entry.formation.density(input) - stock.density[index])
                    .abs()
                    .max_element()
                    < 2e-12,
                "{}/density/{index}",
                stock.id
            );
            assert!(
                (entry.optical_density(input) - stock.od[index])
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
fn density_prefix_is_linear_and_b_translates_before_range_continuation() {
    let stock = spektrafilm::Negative::from_id("kodak_portra_400").unwrap();
    let cancel = AtomicBool::new(false);
    let a = Entry::new(stock, Variant::A, &cancel).unwrap();
    let b = Entry::new(stock, Variant::B, &cancel).unwrap();
    assert_eq!(a.raw_codes(DVec3::ZERO), DVec3::ZERO);
    assert_eq!(b.raw_codes(DVec3::ZERO), DVec3::splat(62.));
    let x = DVec3::new(0.1, 0.5, 0.9);
    let y = DVec3::new(0.8, 0.2, 1.2);
    let t = 0.37;
    let interpolated = a.raw_codes(x * (1. - t) + y * t);
    let expected = a.raw_codes(x) * (1. - t) + a.raw_codes(y) * t;
    assert!((interpolated - expected).abs().max_element() < 1e-12);
    for density in [DVec3::ZERO, x, y, DVec3::splat(4.)] {
        let ca = a.raw_codes(density);
        let cb = b.raw_codes(density);
        let bound = 8. * f64::EPSILON * ca.abs().max_element().max(cb.abs().max_element()).max(62.);
        assert!((cb - ca - DVec3::splat(62.)).abs().max_element() <= bound);
    }
}

#[test]
fn scanner_balances_base_flux_and_only_grey_chroma_for_every_colour_stock() {
    let cancel = AtomicBool::new(false);
    for stock in spektrafilm::Negative::all() {
        let entry = Entry::new(stock, Variant::A, &cancel).unwrap();
        assert_eq!(entry.attenuation(entry.base), DVec3::ZERO);
        let mass = entry
            .bands
            .iter()
            .fold(DVec3::ZERO, |sum, sample| sum + sample.weight);
        assert!((mass - DVec3::ONE).abs().max_element() < 2e-15);
        let gray = entry.optical_density(DVec3::splat(0.18));
        let balanced = gray * entry.balance;
        assert!(
            (balanced - DVec3::splat(gray.element_sum() / 3.))
                .abs()
                .max_element()
                < 1e-14
        );
        for scale in [1e-9, 1e-5, 0.01, 1.] {
            let delta = DVec3::new(scale, scale * 0.7, scale * 0.3);
            let flux = entry.bands.iter().fold(DVec3::ZERO, |sum, sample| {
                sum + sample.weight * 10_f64.powf(-sample.dyes.dot(delta))
            });
            let expected = -flux.map(f64::ln);
            assert!(
                (entry.attenuation(entry.base + delta) - expected)
                    .abs()
                    .max_element()
                    < 2e-13
            );
        }
    }
}
