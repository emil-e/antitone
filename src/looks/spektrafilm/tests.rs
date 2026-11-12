use super::*;
use serde::Deserialize;
#[cfg(feature = "gpu")]
mod gpu;

#[derive(Deserialize)]
struct Fixtures {
    revision: String,
    cases: Vec<Case>,
    paper_only: Vec<PaperCase>,
}
#[derive(Deserialize)]
struct Case {
    negative: String,
    paper: String,
    input: Vec<DVec3>,
    film_h: Vec<DVec3>,
    film_density: Vec<DVec3>,
    paper_h: Vec<DVec3>,
    paper_density: Vec<DVec3>,
    reference_rgb: Vec<DVec3>,
}
#[derive(Deserialize)]
struct Photometry {
    dyes: Vec<DVec3>,
    weights: Vec<f64>,
}
#[derive(Deserialize)]
struct PaperCase {
    photometry: Photometry,
    physical_y: Vec<f64>,
    paper: String,
    input: Vec<DVec3>,
    paper_h: Vec<DVec3>,
    paper_density: Vec<DVec3>,
    reference_rgb: Vec<DVec3>,
}
fn fixtures() -> Fixtures {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/spektrafilm/conformance.json"
    ))
    .unwrap()
}

pub(super) fn paper_luminance(paper: &data::Paper, h: DVec3) -> f64 {
    static SOURCE: std::sync::OnceLock<Fixtures> = std::sync::OnceLock::new();
    let case = SOURCE
        .get_or_init(fixtures)
        .paper_only
        .iter()
        .find(|case| case.paper == paper.id)
        .expect("source photometry for every medium");
    let density = develop_print_morph(paper, h);
    case.photometry
        .dyes
        .iter()
        .zip(&case.photometry.weights)
        .map(|(dye, weight)| weight * 10_f64.powf(-dye.dot(density)))
        .sum()
}

#[test]
fn source_evaluated_catalogue_has_every_selectable_pair_and_preserves_identity() {
    let f = fixtures();
    assert_eq!(f.revision, REVISION);
    let expected: std::collections::BTreeSet<_> = Negative::all()
        .flat_map(|n| Paper::all().map(move |p| (n.id().to_owned(), p.id().to_owned())))
        .collect();
    let actual = f
        .cases
        .iter()
        .map(|v| (v.negative.clone(), v.paper.clone()))
        .collect();
    assert_eq!(expected, actual);
    let expected_papers: std::collections::BTreeSet<_> =
        Paper::all().map(|p| p.id().to_owned()).collect();
    let actual_papers = f.paper_only.iter().map(|p| p.paper.clone()).collect();
    assert_eq!(expected_papers, actual_papers);
    assert_eq!(f.paper_only.len(), Paper::all().len());
    for case in &f.paper_only {
        assert_eq!(case.physical_y.len(), case.input.len());
        assert!(!case.photometry.weights.is_empty());
        assert_eq!(case.photometry.dyes.len(), case.photometry.weights.len());
    }
    for case in &f.cases {
        assert_eq!(case.reference_rgb.len(), case.input.len());
        assert!(case.reference_rgb.iter().all(|v| v.is_finite()));
    }
    for n in Negative::all() {
        assert_eq!(Negative::from_id(n.id()), Some(n));
    }
    for paper in Paper::all() {
        for negative in [None, Some(Negative::from_id("kodak_portra_400").unwrap())] {
            let s = Selection { negative, paper };
            assert_eq!(Selection::from_origin(&s.origin()), Some(s));
        }
    }
}

#[test]
fn native_formation_and_spectral_projection_match_executed_upstream() {
    let cases = fixtures().cases;
    let cancel = AtomicBool::new(false);
    let catalog = data::catalog();
    let mut max_input: f64 = 0.;
    let mut max_density: f64 = 0.;
    let mut max_projection: f64 = 0.;
    let mut max_approximation: f64 = 0.;
    for n in Negative::all() {
        let native = Filming::new(n, Paper::default(), &cancel).unwrap();
        for case in cases.iter().filter(|case| case.negative == n.id()) {
            let pair = catalog
                .pairs
                .iter()
                .find(|p| p.negative == case.negative && p.paper == case.paper)
                .unwrap();
            for (i, input) in case.input.iter().enumerate() {
                let h = native.expose(*input);
                max_input = max_input.max((h - case.film_h[i]).abs().max_element());
                let d = develop(native.profile, h);
                max_density = max_density.max((d - case.film_density[i]).abs().max_element());
                let ph = film_cmy_to_print_log_raw(native.profile, pair, d);
                max_projection = max_projection.max((ph - case.paper_h[i]).abs().max_element());
                let paper = &catalog.paper[Paper::from_id(&case.paper).unwrap().0];
                assert!(
                    (develop_print_morph(paper, ph) - case.paper_density[i])
                        .abs()
                        .max_element()
                        < 1e-10
                );
                if case.paper == Paper::default().id() {
                    max_approximation =
                        max_approximation.max((native.printing.sample(d) - ph).abs().max_element());
                }
            }
        }
    }
    println!(
        "source input {max_input}; development {max_density}; spectral projection {max_projection}; generated33 printing logH error {max_approximation}"
    );
    assert!(max_input < 1e-10);
    assert!(max_density < 1e-10);
    assert!(max_projection < 1e-10);
    assert!(max_approximation < 1e-5);
}

#[test]
fn paper_only_exposure_and_density_match_executed_source_without_film_formation() {
    for case in fixtures().paper_only {
        assert_eq!(case.reference_rgb.len(), case.input.len());
        assert!(case.reference_rgb.iter().all(|v| v.is_finite()));
        let selection = Selection {
            negative: None,
            paper: Paper::from_id(&case.paper).unwrap(),
        };
        let model = Model::new(selection, &AtomicBool::new(false)).unwrap();
        for (i, input) in case.input.iter().enumerate() {
            let h = model.paper_exposure(*input).unwrap();
            assert!((h - case.paper_h[i]).abs().max_element() < 1e-12);
            let d = develop_print_morph(model.paper, h);
            assert!((d - case.paper_density[i]).abs().max_element() < 1e-12);
            assert!((paper_luminance(model.paper, h) - case.physical_y[i]).abs() < 2e-14);
            assert!(model.encoded(*input).unwrap().is_finite());
        }
        let neutral = model
            .encoded(DVec3::splat(0.18))
            .unwrap()
            .map(photographic::decode);
        println!("{} paper-only gray {neutral:?}", case.paper);
        assert!((neutral - DVec3::splat(0.18)).abs().max_element() < 0.001);
    }
}

#[test]
fn every_media_reference_retains_neutral_tone_and_allows_colour_dependent_luminance() {
    let mapping = &data::catalog().paper_mapping;
    for paper in Paper::all() {
        let model = Model::new(
            Selection {
                negative: None,
                paper,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(model.film.is_none());
        let driver = &model.paper.positive_driver;
        let baseline = |brightness| {
            let offset = mapping.exposure_compression
                * 17.52
                * (photographic::encode(brightness) - photographic::encode(mapping.gray_ap1))
                * std::f64::consts::LOG10_2;
            model.paper.positive_gray_log_h - DVec3::splat(offset)
        };
        for value in [-1., -0.02, 0., 0.003, 0.0078125, 0.18, 1., 16., 1024.] {
            let expected = baseline(value);
            let actual = model.paper_exposure(DVec3::splat(value)).unwrap();
            assert!(
                (actual - expected).abs().max_element() < 1e-12,
                "{}",
                paper.id()
            );
            assert_eq!(
                model.encoded(DVec3::splat(value)).unwrap(),
                model
                    .scanner
                    .sample(develop_print_morph(model.paper, actual))
            );
        }
        for colour in [DVec3::new(0.32, 0.16, 0.10), DVec3::new(0.05, 0.18, 0.30)] {
            let neutral = baseline(driver.input_luminance.dot(colour));
            let actual = model.paper_exposure(colour).unwrap();
            assert!(
                (paper_luminance(model.paper, actual) - paper_luminance(model.paper, neutral))
                    .abs()
                    > 1e-6,
                "{}",
                paper.id()
            );
            let difference = actual - neutral;
            assert!(
                (difference - DVec3::splat(difference.x))
                    .abs()
                    .max_element()
                    > 1e-6,
                "{}",
                paper.id()
            );
            assert!(
                (model.encoded(colour).unwrap()
                    - model
                        .scanner
                        .sample(develop_print_morph(model.paper, neutral)))
                .abs()
                .max_element()
                    > 1e-5,
                "{}",
                paper.id()
            );
        }
    }
}

#[test]
fn every_media_record_requires_its_reference_driver() {
    let mut value: serde_json::Value =
        serde_json::from_slice(&data::decompress("catalog.json.z").unwrap()).unwrap();
    for i in 0..value["paper"].as_array().unwrap().len() {
        let driver = value["paper"][i]
            .as_object_mut()
            .unwrap()
            .remove("positive_driver")
            .unwrap();
        assert!(serde_json::from_value::<data::Catalog>(value.clone()).is_err());
        value["paper"][i]["positive_driver"] = driver;
    }
}

#[test]
fn cubic_sampler_preserves_cross_terms_and_nonfinite_input_rejects() {
    let size = 5;
    let mut values = Vec::new();
    for i in 0..size {
        for j in 0..size {
            for k in 0..size {
                let x = i as f64 / 4.;
                let y = j as f64 / 4.;
                let z = k as f64 / 4.;
                values.push(DVec3::new(x * y, x * y * z, x * x * x + z));
            }
        }
    }
    let grid = CubicGrid {
        size,
        min: DVec3::ZERO,
        max: DVec3::ONE,
        values,
    };
    let result = grid.sample(DVec3::new(0.3, 0.7, 0.2));
    assert!(
        (result - DVec3::new(0.21, 0.042, 0.227))
            .abs()
            .max_element()
            < 1e-14
    );
    let model = Model::new(
        Selection {
            negative: None,
            paper: Paper::default(),
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(model.encoded(DVec3::splat(f64::NAN)).is_err());
    assert!(model.encoded(DVec3::splat(f64::INFINITY)).is_err());
}

#[test]
fn cancellation_and_baked_sample_order_are_explicit() {
    let cancel = AtomicBool::new(false);
    let model = Model::new(
        Selection {
            negative: None,
            paper: Paper::default(),
        },
        &cancel,
    )
    .unwrap();
    let asset = model.bake(5, &cancel).unwrap();
    let lut = asset.lut();
    let (lo, hi) = lut.domain();
    for (r, g, b) in [(0, 0, 0), (4, 0, 0), (0, 4, 0), (0, 0, 4), (2, 3, 1)] {
        let input = DVec3::new(r as f64, g as f64, b as f64).map(|v| {
            photographic::decode(f64::from(lo[0]) + (f64::from(hi[0]) - f64::from(lo[0])) * v / 4.)
        });
        let expected = model.encoded(input).unwrap();
        let actual = DVec3::from_array(lut.nodes()[(b * 5 + g) * 5 + r].map(f64::from));
        assert!((actual - expected).abs().max_element() < 2e-6);
    }
    cancel.store(true, Ordering::Relaxed);
    assert!(model.bake(5, &cancel).is_err());
    assert!(
        Model::new(
            Selection {
                negative: Some(Negative::all().next().unwrap()),
                paper: Paper::default()
            },
            &cancel
        )
        .is_err()
    );
}
