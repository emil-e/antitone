use super::*;
mod provenance;

#[test]
fn authoring_response_matches_independent_density_controls() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/frontier/density-controls.json"
    ))
    .unwrap();
    let inputs = fixture["inputs"].as_array().unwrap();
    assert_eq!(inputs.len(), 151);
    for (variant, key) in [(Variant::A, "A"), (Variant::B, "B")] {
        let model = Model::new(
            Selection {
                negative: None,
                variant,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let expected = fixture["outputs"][key]["AP1"].as_array().unwrap();
        assert_eq!(inputs.len(), expected.len());
        for (index, (input, expected)) in inputs.iter().zip(expected).enumerate() {
            let rgb = DVec3::from_array(std::array::from_fn(|c| input[c].as_f64().unwrap()));
            let reference =
                DVec3::from_array(std::array::from_fn(|c| expected[c].as_f64().unwrap()));
            let Input::Positive(entry) = &model.input else {
                unreachable!()
            };
            let codes = &fixture["outputs"][key]["codes"][index];
            let codes = DVec3::from_array(std::array::from_fn(|c| codes[c].as_f64().unwrap()));
            assert!(
                (entry.coordinates(rgb).unwrap() - codes)
                    .abs()
                    .max_element()
                    < 1e-10,
                "{key}/{index}/codes"
            );
            assert!(
                (model.ap1(rgb).unwrap() - reference).abs().max_element() < 2e-13,
                "{key}/{index}"
            );
        }
    }
}

#[test]
fn preparation_uses_shared_grid_and_explicit_encoded_ap1_output() {
    let cancel = AtomicBool::new(false);
    for variant in Variant::all() {
        let selection = Selection {
            negative: None,
            variant,
        };
        let model = Model::new(selection, &cancel).unwrap();
        let small = super::super::material::Selection::Frontier(selection)
            .bake(3, &cancel)
            .unwrap();
        assert_eq!(small.lut().size(), 3);
        super::super::sampling::assert_material_vertices(small.lut(), |rgb| {
            Ok(model.ap1(rgb)?.map(photographic::encode))
        });
        let default = selection.prepare(&cancel).unwrap();
        assert_eq!(default.lut().size(), baking::DEFAULT_GRID);
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/frontier/density-controls.json"
        ))
        .unwrap();
        let key = if variant == Variant::A { "A" } else { "B" };
        assert_eq!(
            default.id().to_string(),
            fixture["outputs"][key]["lut_identity"].as_str().unwrap()
        );
        for input in [DVec3::ZERO, DVec3::splat(-0.01)] {
            let sampled = default.lut().apply(input).unwrap();
            if variant == Variant::A {
                // Stored ACEScct zero is rounded to f32; a double-precision
                // decode therefore has a tiny quantization residual, not a pedestal.
                let zero = f64::from(photographic::encode(0.) as f32);
                let encoded = default
                    .lut()
                    .sample_encoded(input.map(photographic::encode))
                    .unwrap();
                assert!(
                    (encoded - DVec3::splat(zero)).abs().max_element() <= 4. * f64::EPSILON * zero
                );
            } else {
                assert!(sampled.min_element() > 0.);
            }
        }
        super::super::sampling::assert_material_vertices(default.lut(), |rgb| {
            Ok(model.ap1(rgb)?.map(photographic::encode))
        });
        assert_ne!(small.id(), default.id());
        assert_eq!(small.origin(), default.origin());
        assert!(model.bake(1, &cancel).is_err());
        assert!(
            model
                .bake(65, &AtomicBool::new(true))
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }
}

#[test]
fn fixed_black_anchor_uses_each_source_bank_without_an_output_black_override() {
    let cancel = AtomicBool::new(false);
    for negative in [None]
        .into_iter()
        .chain(spektrafilm::Negative::all().map(Some))
    {
        for variant in Variant::all() {
            let model = Model::new(Selection { negative, variant }, &cancel).unwrap();
            let source = match negative {
                Some(_) => density::Source::Negative(variant),
                None => density::Source::Native(variant),
            };
            let black = model.ap1(DVec3::ZERO).unwrap();
            assert_eq!(
                black,
                source
                    .load()
                    .unwrap()
                    .sample(DVec3::splat(variant.code_offset()))
            );
            if variant == Variant::A {
                assert_eq!(black, DVec3::ZERO);
            } else {
                assert!(black.min_element() > 0.);
            }
        }
    }
}

#[test]
fn density_source_interpolation_reproduces_nonseparable_affine_data() {
    let nodes = (0..density::NODES)
        .map(|i| {
            let r = (i % 65) as f32 / 64.;
            let g = ((i / 65) % 65) as f32 / 64.;
            let b = (i / 4225) as f32 / 64.;
            [
                0.125 + 0.5 * r + 0.25 * g,
                0.25 + 0.125 * g + 0.5 * b,
                -0.125 + 0.25 * r + 0.5 * b,
            ]
        })
        .collect();
    let response = density::Response::new(nodes).unwrap();
    for r in [0., 0.1, 0.5, 1.] {
        for g in [0., 0.17, 1.] {
            for b in [0., 0.73, 1.] {
                let expected = DVec3::new(
                    0.125 + 0.5 * r + 0.25 * g,
                    0.25 + 0.125 * g + 0.5 * b,
                    -0.125 + 0.25 * r + 0.5 * b,
                );
                assert!(
                    (response.sample(DVec3::new(r, g, b) * 1023.) - expected)
                        .abs()
                        .max_element()
                        < 2e-15
                );
            }
        }
    }
}

#[test]
fn generated_cache_respects_requested_grid_for_every_provider() {
    let directory = tempfile::tempdir().unwrap();
    let cache = super::super::cache::Cache::at(directory.path().into());
    let selection = super::super::material::Selection::Frontier(Selection {
        negative: None,
        variant: Variant::B,
    });
    let asset = selection.bake(3, &AtomicBool::new(false)).unwrap();
    cache.store_photochemical(selection, &asset).unwrap();
    assert_eq!(cache.load_photochemical(selection, 3).unwrap(), Some(asset));
    assert_eq!(cache.load_photochemical(selection, 5).unwrap(), None);
}
