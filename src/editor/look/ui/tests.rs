use super::*;
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
fn historic_builtin_sources_use_catalog_labels_but_imports_keep_their_names() {
    let data = asset();
    for (builtin, name, old_name) in [
        (Builtin::PortraEndura, "Portrait Paper", "Portra Endura"),
        (Builtin::ProImageII, "Colour Paper", "Pro Image II"),
        (Builtin::Kodak2383, "Cinema Print", "Kodak 2383"),
        (
            Builtin::IlfordFilter2,
            "Monochrome Paper",
            "Ilford RC Deluxe · Filter 2",
        ),
    ] {
        let legacy = LookAsset::new(
            old_name.into(),
            LookOrigin::Builtin {
                id: builtin.id().into(),
                version: 73,
                generator: "historic-generator".into(),
                definition: "a".repeat(64),
            },
            data.lut().clone(),
        )
        .unwrap();
        assert_eq!(asset_label(&legacy), name);
        assert_eq!(builtin_for_asset(&legacy), Some(builtin));
        let note = builtin_attribution(builtin);
        assert!(note.contains(builtin.source_product()));
        assert!(note.contains("Independent creative interpretation"));
        assert!(note.contains("Not affiliated with or endorsed"));
    }
    let imported = LookAsset::new(
        "Kodak 2383".into(),
        LookOrigin::Imported,
        data.lut().clone(),
    )
    .unwrap();
    assert_eq!(asset_label(&imported), "Kodak 2383");
    assert_eq!(builtin_for_asset(&imported), None);
    let unknown = LookAsset::new(
        "Saved historic look".into(),
        LookOrigin::Builtin {
            id: "unknown-source".into(),
            version: 1,
            generator: "historic-generator".into(),
            definition: "a".repeat(64),
        },
        data.lut().clone(),
    )
    .unwrap();
    assert_eq!(asset_label(&unknown), "Saved historic look");
    assert_eq!(builtin_for_asset(&unknown), None);
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
        ProcessingInput::from(ProcessingParameters::source()).with_look(Some(look.clone()));
    assert!(source.look().is_none());
    let calibration = Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.003),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    };
    let input = ProcessingInput::from(
        ProcessingParameters::positive(calibration, RollSettings::default(), glam::DMat3::IDENTITY)
            .unwrap(),
    );
    let looked = input.clone().with_look(Some(look));
    assert_ne!(input.signature(), looked.signature());
    assert!(input.signature().same_photographic(&looked.signature()));
}
