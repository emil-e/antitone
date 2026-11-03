use super::*;
use std::io::Read;

#[derive(Deserialize)]
pub(super) struct Cases {
    pub inputs: Vec<DVec3>,
    pub cases: Vec<Case>,
}
#[derive(Deserialize)]
pub(super) struct Case {
    negative: Option<String>,
    paper: String,
    raw: Option<Vec<f64>>,
    film_log_h: Option<Vec<f64>>,
    film_density: Option<Vec<f64>>,
    pub log_h: Vec<f64>,
    pub density: Vec<f64>,
    pub encoded: Vec<DVec3>,
    pub reference_rgb: Vec<DVec3>,
}
impl Case {
    pub fn selection(&self) -> Selection {
        Selection {
            negative: self
                .negative
                .as_ref()
                .map(|id| Negative::from_id(id).unwrap()),
            paper: Paper::from_id(&self.paper).unwrap(),
        }
    }
}
pub(super) fn fixtures() -> Cases {
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(
        include_bytes!("../../../../tests/fixtures/monochrome/conformance.json.z").as_slice(),
    )
    .read_to_end(&mut bytes)
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn all_source_materials_and_pairs_match_executed_projection_development_and_readout() {
    let fixtures = fixtures();
    let count = fixtures.inputs.len();
    assert!(count > 1000);
    assert_eq!(
        fixtures.cases.len(),
        Paper::all().len() * (Negative::all().len() + 1)
    );
    let mut seen = std::collections::HashSet::new();
    let mut max_encoded: f64 = 0.;
    let mut max_h: f64 = 0.;
    for case in &fixtures.cases {
        assert!(seen.insert((case.negative.clone(), case.paper.clone())));
        let model = Model::new(case.selection());
        for values in [&case.encoded, &case.reference_rgb] {
            assert_eq!(count, values.len());
        }
        assert_eq!(count, case.log_h.len());
        assert_eq!(count, case.density.len());
        if let Some(film) = &model.film {
            let raw = case.raw.as_ref().unwrap();
            let log_h = case.film_log_h.as_ref().unwrap();
            let density = case.film_density.as_ref().unwrap();
            assert_eq!(count, raw.len());
            assert_eq!(count, log_h.len());
            assert_eq!(count, density.len());
            for i in 0..count {
                // Generated f32 exposure coefficients have at most one-ulp
                // relative rounding; the upstream reference retains f64 values.
                assert!(
                    (film.raw(fixtures.inputs[i]) - raw[i]).abs() < 1e-7 * raw[i].abs().max(1.),
                    "{} raw case {i}",
                    model.selection.name()
                );
                assert!((film.exposure(fixtures.inputs[i]) - log_h[i]).abs() < 1e-7);
                assert!((film.profile.exposure.sample(log_h[i]) - density[i]).abs() < 2e-12);
            }
        } else {
            assert!(case.raw.is_none() && case.film_log_h.is_none() && case.film_density.is_none());
        }
        for i in 0..count {
            max_h = max_h.max((model.exposure(fixtures.inputs[i]) - case.log_h[i]).abs());
            assert!((model.density(case.log_h[i]) - case.density[i]).abs() < 2e-12);
            max_encoded = max_encoded.max(
                (model.encoded(fixtures.inputs[i]).unwrap() - case.encoded[i])
                    .abs()
                    .max_element(),
            );
        }
    }
    println!(
        "{} material combinations × {count} source probes; max paper logH {max_h}, encoded {max_encoded}",
        fixtures.cases.len()
    );
    assert!(max_h < 1e-7);
    // Includes dense source/readout interpolation error (<1e-6),
    // table packing and fixed inverse-authoring DRT f32 roundoff.
    assert!(max_encoded < 3e-6);
}

#[test]
fn spectral_negatives_remain_scalar_exposures_and_supported_pairings_are_structural() {
    let gray = DVec3::splat(0.18);
    let weights = color::AP1_TO_XYZ.row(1);
    let equal_y = gray + DVec3::new(0.2, -0.2 * weights.x / weights.y, 0.);
    for negative in Negative::all() {
        let film = Filming::new(negative);
        assert!((film.raw(gray) - 1.).abs() < 1e-7);
        assert!((film.raw(gray * 2.) - 2. * film.raw(gray)).abs() < 1e-12);
        assert!(
            (film.exposure(gray * 2.) - film.exposure(gray) - std::f64::consts::LOG10_2).abs()
                < 1e-9
        );
        // Equal AP1 luminance is not equal film exposure: this rejects silently
        // applying the paper-only luminance adapter to synthetic negatives.
        assert!((film.exposure(gray) - film.exposure(equal_y)).abs() > 0.01);
        let curve = &film.profile.exposure;
        assert_eq!(curve.sample(curve.min - 100.), curve.density[0]);
        assert_eq!(
            curve.sample(curve.max + 100.),
            *curve.density.last().unwrap()
        );
        for paper in Paper::all() {
            let selection = Selection {
                negative: Some(negative),
                paper,
            };
            assert_eq!(Selection::from_origin(&selection.origin()), Some(selection));
            let facade =
                crate::looks::material::Selection::new(Some(negative.into()), paper.into())
                    .unwrap();
            assert_eq!(facade, selection.into());
            assert!(
                crate::looks::material::Selection::new(
                    Some(negative.into()),
                    crate::looks::spektrafilm::Paper::default().into()
                )
                .is_none()
            );
        }
    }
    let colour = crate::looks::spektrafilm::Negative::from_id("kodak_portra_400").unwrap();
    for paper in Paper::all() {
        assert!(
            crate::looks::material::Selection::new(Some(colour.into()), paper.into()).is_none()
        );
    }
}

#[test]
fn paper_black_density_follows_each_published_curve_without_common_endpoint_adjustment() {
    for (id, published_max) in [
        ("ilford_multigrade_rc_deluxe_filter_2", 2.1),
        ("foma_fomaspeed_variant_glossy_filter_2", 2.1),
        ("foma_fomabrom_variant_glossy_filter_2", 2.0),
    ] {
        let model = Model::new(Paper::from_id(id).unwrap());
        let black = model.density(model.paper.exposure.max + 1.);
        assert!((black - published_max).abs() < 0.025, "{id} source Dmax");
        assert!(model.density(model.paper.exposure.min - 1.) < 0.15);
    }
}
