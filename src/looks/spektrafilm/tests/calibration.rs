use super::*;
use crate::rendering::{CpuRenderer, OutputTarget};

#[test]
#[ignore = "Set ANTITONE_PAPER_REFERENCE_ASSETS to the accepted calibration's eight65 JSON assets"]
fn production_paper_bakes_preserve_the_accepted_calibration() {
    let folder = std::path::PathBuf::from(
        std::env::var_os("ANTITONE_PAPER_REFERENCE_ASSETS").expect("accepted calibration assets"),
    );
    let output = std::env::var_os("ANTITONE_PAPER_OUTPUT_ASSETS").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir_all(path).unwrap();
    }
    let cancel = AtomicBool::new(false);
    let mut inputs = vec![];
    for r in [-0.03, 0., 0.003, 0.02, 0.18, 0.8, 4., 32.] {
        for g in [-0.02, 0.005, 0.18, 0.5, 2.] {
            for b in [-0.04, 0., 0.0078125, 0.12, 0.25, 0.42, 4.] {
                inputs.push(DVec3::new(r, g, b));
            }
        }
    }
    for ev in -24..=24 {
        for b in [0.12, 0.2084451318, 0.2823168039, 0.42] {
            inputs.push(DVec3::new(3.8218507767, 1.8416701555, b) * 2_f64.powf(ev as f64 / 3.));
        }
    }
    for paper in Paper::all() {
        let name = format!("{}.json", paper.id());
        let expected: LookAsset =
            serde_json::from_slice(&std::fs::read(folder.join(&name)).unwrap()).unwrap();
        let model = Model::new(
            Selection {
                negative: None,
                paper,
            },
            &cancel,
        )
        .unwrap();
        let actual = model
            .bake(crate::looks::authoring::DEFAULT_GRID, &cancel)
            .unwrap();
        assert_eq!(actual.lut().size(), 65);
        assert_eq!(expected.lut().size(), actual.lut().size());
        assert_eq!(actual.lut().domain(), expected.lut().domain());
        let node_error = actual
            .lut()
            .nodes()
            .iter()
            .zip(expected.lut().nodes())
            .flat_map(|(a, b)| (0..3).map(move |c| (a[c] as f64 - b[c] as f64).abs()))
            .fold(0_f64, f64::max);
        assert!(
            node_error < 1e-6,
            "{} changed calibration nodes: {node_error}",
            paper.id()
        );
        let actual_carriers: Vec<_> = inputs
            .iter()
            .map(|&p| actual.lut().apply(p).unwrap())
            .collect();
        let expected_carriers: Vec<_> = inputs
            .iter()
            .map(|&p| expected.lut().apply(p).unwrap())
            .collect();
        for target in [OutputTarget::DisplayP3, OutputTarget::Srgb] {
            let renderer = CpuRenderer::new(target).unwrap();
            let a = target
                .encode_u16(&renderer.render(&actual_carriers).unwrap())
                .unwrap();
            let b = target
                .encode_u16(&renderer.render(&expected_carriers).unwrap())
                .unwrap();
            let error = a
                .iter()
                .zip(b)
                .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c]) as f64 / 257.))
                .fold(0_f64, f64::max);
            println!(
                "{} {target:?}: {} accepted/production sites, maxRGB8difference {error}, maxnode {node_error}",
                paper.id(),
                inputs.len()
            );
            assert!(
                error < 0.02,
                "changed accepted calibration: {} {error}",
                paper.id()
            );
        }
        if let Some(path) = &output {
            std::fs::write(path.join(name), serde_json::to_vec(&actual).unwrap()).unwrap();
        }
    }
}
