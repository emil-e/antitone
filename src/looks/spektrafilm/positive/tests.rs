use super::*;
use crate::looks::spektrafilm::{Input, Model, Paper, Selection, develop_print_morph};
use crate::photographic;
use std::sync::atomic::AtomicBool;

fn model(paper: Paper) -> Model {
    Model::new(
        Selection {
            negative: None,
            paper,
        },
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn models() -> impl Iterator<Item = Model> {
    Paper::all().map(model)
}

#[test]
fn printing_balance_is_fixed_and_all_inputs_use_the_same_spectral_reference() {
    for m in models() {
        let Input::Reference(driver) = &m.input else {
            panic!("paper-only reference");
        };
        let gray = crate::looks::characteristic::reference().gray;
        assert!(
            (m.paper_exposure(DVec3::splat(gray)).unwrap() - m.paper.positive_gray_log_h)
                .abs()
                .max_element()
                < 2e-12
        );
        for rgb in [
            DVec3::ZERO,
            DVec3::splat(-0.02),
            DVec3::splat(0.003),
            DVec3::splat(gray),
            DVec3::splat(4.),
            DVec3::new(0.3, 0.1, 0.02),
        ] {
            let reference = driver.reference.expose(rgb, driver.pair);
            assert!(
                (m.paper_exposure(rgb).unwrap() - reference - driver.print_log_h)
                    .abs()
                    .max_element()
                    < 1e-12
            );
        }
        let low = m.paper_exposure(DVec3::splat(0.03)).unwrap();
        let high = m.paper_exposure(DVec3::splat(0.3)).unwrap();
        let change = high - low;
        assert!((change - DVec3::splat(change.x)).abs().max_element() > 1e-4);
        for rgb in [
            DVec3::new(-0.0212899204, 0.0399119705, -0.0189858880),
            DVec3::new(-100., 1., 1024.),
            DVec3::new(1e100, -1e100, 0.),
            DVec3::new(1e-100, -2e-100, 3e-100),
        ] {
            assert!(m.paper_exposure(rgb).unwrap().is_finite(), "{}", m.paper.id);
        }
    }
}

#[test]
fn complete_neutral_trajectory_is_finite_monotone_and_continuous_at_black() {
    let mut values: Vec<_> = (0..=4096)
        .map(|i| photographic::decode(-0.5 + 1.6 * i as f64 / 4096.))
        .collect();
    values.extend([-100., -1., 0., 0.0078125, 0.18, 1024., 1e6]);
    values.sort_by(f64::total_cmp);
    for m in models() {
        let exposures: Vec<_> = values
            .iter()
            .map(|&x| m.paper_exposure(DVec3::splat(x)).unwrap())
            .collect();
        assert!(exposures.iter().all(|x| x.is_finite()));
        for pair in exposures.windows(2) {
            assert!(
                pair[1].cmple(pair[0] + DVec3::splat(1e-12)).all(),
                "{}",
                m.paper.id
            );
        }
        let black = m.paper_exposure(DVec3::ZERO).unwrap();
        assert!(
            (m.paper_exposure(DVec3::splat(1e-10)).unwrap() - black)
                .abs()
                .max_element()
                < 1e-12
        );
    }
}

#[test]
fn signed_and_hdr_colours_remain_finite_without_disabling_colour() {
    let luminance = crate::color::AP1_TO_XYZ.row(1);
    let direction = DVec3::new(0.7, -0.4, 1.2);
    let opponent = direction - DVec3::splat(luminance.dot(direction));
    for m in models() {
        for brightness in [-0.1, 0., 0.002, 0.02, 0.18, 0.8, 4., 1024.] {
            for amount in [-2., -0.05, 0., 0.03, 2.] {
                let input = DVec3::splat(brightness) + opponent * amount;
                assert!(
                    m.paper_exposure(input).unwrap().is_finite(),
                    "{}",
                    m.paper.id
                );
                assert!(m.encoded(input).unwrap().is_finite(), "{}", m.paper.id);
            }
        }
        let neutral = m.encoded(DVec3::splat(0.18)).unwrap();
        let coloured = m.encoded(DVec3::splat(0.18) + opponent * 0.03).unwrap();
        assert!(
            (coloured - neutral).abs().max_element() > 0.005,
            "{}",
            m.paper.id
        );
    }
}

#[test]
fn only_nonpositive_exposure_records_share_black_and_warm_hdr_queries_remain_defined() {
    for m in models() {
        let black = m.paper_exposure(DVec3::ZERO).unwrap();
        assert_eq!(m.paper_exposure(DVec3::splat(-1.)).unwrap(), black);
        // Negative input luminance does not discard independently positive records.
        let signed = DVec3::new(0.01, -0.05, 0.02);
        assert!((crate::color::AP1_TO_XYZ * signed).y < 0.);
        assert!(
            (m.paper_exposure(signed).unwrap() - black)
                .abs()
                .max_element()
                > 1e-4
        );
        for input in [
            DVec3::new(32., 17.6, 0.2),
            DVec3::new(22.255227981125067, 12.411862377076536, -0.05435413775647024),
        ] {
            assert!(
                m.paper_exposure(input).unwrap().is_finite(),
                "{}",
                m.paper.id
            );
            assert!(m.encoded(input).unwrap().is_finite(), "{}", m.paper.id);
        }
    }
}

#[test]
#[ignore = "Set ANTITONE_MEDIA_PAPER, ANTITONE_MEDIA_POINTS (f32 AP1), ANTITONE_MEDIA_EXPOSURES (f64 source logH)"]
fn captured_sites_match_source_exposure_density_and_spectral_y() {
    let inputs =
        std::fs::read(std::env::var_os("ANTITONE_MEDIA_POINTS").expect("captured AP1 points"))
            .unwrap();
    let expected = std::fs::read(
        std::env::var_os("ANTITONE_MEDIA_EXPOSURES").expect("independent source exposures"),
    )
    .unwrap();
    let paper = std::env::var("ANTITONE_MEDIA_PAPER").expect("source paper ID");
    let m = model(Paper::from_id(&paper).expect("known source paper"));
    assert!(!inputs.is_empty() && inputs.len().is_multiple_of(12));
    assert!(expected.len().is_multiple_of(24));
    assert_eq!(inputs.len() / 12, expected.len() / 24);
    let mut raw: f64 = 0.;
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
        density = density.max(
            (develop_print_morph(m.paper, actual) - develop_print_morph(m.paper, source))
                .abs()
                .max_element(),
        );
        luminance = luminance.max(
            (super::super::tests::paper_luminance(m.paper, actual)
                - super::super::tests::paper_luminance(m.paper, source))
            .abs(),
        );
    }
    println!(
        "{paper}: {} source poses: logH {raw}, density {density}, spectral Y {luminance}",
        inputs.len() / 12
    );
    assert!(raw < 1e-9);
    assert!(density < 1e-9);
    assert!(luminance < 2e-12);
}
