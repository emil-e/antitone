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
struct PaperCase {
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
fn paper_only_has_no_film_formation_and_retains_the_positive_exposure_convention() {
    for case in fixtures().paper_only {
        assert_eq!(case.reference_rgb.len(), case.input.len());
        assert!(case.reference_rgb.iter().all(|v| v.is_finite()));
        let selection = Selection {
            negative: None,
            paper: Paper::from_id(&case.paper).unwrap(),
        };
        let model = Model::new(selection, &AtomicBool::new(false)).unwrap();
        for (i, input) in case.input.iter().enumerate() {
            let h = model.paper_exposure(*input);
            assert!((h - case.paper_h[i]).abs().max_element() < 1e-12);
            let d = develop_print_morph(model.paper, h);
            assert!((d - case.paper_density[i]).abs().max_element() < 1e-12);
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
