use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::looks::{LookOrigin, Lut3D};

fn asset() -> Arc<LookAsset> {
    let mut nodes = Vec::new();
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                nodes.push([r as f32, g as f32, b as f32]);
            }
        }
    }
    Arc::new(
        LookAsset::new(
            "Embedded test look".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    )
}

#[test]
fn loaded_menu_names_are_unique_without_relabelling_the_accepted_assets() {
    let first = asset();
    let mut nodes = first.lut().nodes().to_vec();
    nodes[0][0] += 0.01;
    let second = Arc::new(
        LookAsset::new(
            first.name().into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    );
    assert_eq!(
        loaded_lut_labels(&[first.clone(), second.clone()]),
        ["Embedded test look", "Embedded test look (2)"]
    );
    assert_eq!(first.name(), "Embedded test look");
    assert_eq!(second.name(), "Embedded test look");
    let reserved =
        Arc::new(LookAsset::new("None".into(), LookOrigin::Imported, first.lut().clone()).unwrap());
    let collision = Arc::new(
        LookAsset::new(
            "None (2)".into(),
            LookOrigin::Imported,
            second.lut().clone(),
        )
        .unwrap(),
    );
    assert_eq!(
        loaded_lut_labels(&[reserved.clone(), collision, reserved]),
        ["None (2)", "None (2) (2)", "None (3)"]
    );
}

#[test]
fn positive_input_adopts_look_but_source_inspection_cannot() {
    let look = asset();
    let source =
        AuthoredPhotograph::from(ProcessingParameters::source()).with_luts(Some(look.clone()));
    assert!(source.luts().is_empty());
    let calibration = Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.003),
    };
    let input = AuthoredPhotograph::from(
        ProcessingParameters::positive(calibration, RollSettings::default(), glam::DMat3::IDENTITY)
            .unwrap(),
    );
    let looked = input.clone().with_luts(Some(look));
    assert_ne!(input.clone(), looked.clone());
    assert!(input.clone().same_photographic(&looked.clone()));
}
