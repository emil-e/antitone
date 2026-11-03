#![cfg(feature = "project")]
use antitone::{
    looks::{LookAsset, LookOrigin, Lut3D},
    project::{Project, Save},
};
use std::sync::Arc;

fn lut(origin: LookOrigin) -> Arc<LookAsset> {
    Arc::new(
        LookAsset::new(
            "Embedded treatment".into(),
            origin,
            Arc::new(Lut3D::new(3, [-0.5; 3], [1.; 3], vec![[0.4; 3]; 27]).unwrap()),
        )
        .unwrap(),
    )
}
fn material() -> Arc<LookAsset> {
    lut(LookOrigin::Photochemical {
        negative: Some("historic_stock".into()),
        paper: "historic_paper".into(),
        revision: "a".repeat(40),
        generator: "retired-generator".into(),
        attribution: "Retained source attribution".into(),
    })
}

#[test]
fn independent_embedded_slots_survive_save_as_without_catalogue_or_source_files() {
    let mut project = Project::default();
    let default = serde_json::to_value(&project).unwrap();
    for name in ["lut", "film_and_paper", "film_preset", "film_emulation"] {
        assert!(default["output"].get(name).is_none());
    }
    assert_eq!(serde_json::from_value::<Project>(default).unwrap(), project);
    project.output.lut = Some(lut(LookOrigin::Imported));
    project.output.film_and_paper = Some(material());
    project.output.grain = antitone::grain::GrainPreset::MediumFormat.settings();
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.antitone");
    project.save(&original, Save::New).unwrap();
    let restored = Project::load(&original).unwrap();
    assert_eq!(restored, project);
    let value = serde_json::to_value(&restored).unwrap();
    assert!(value["output"].get("film_preset").is_none());
    assert!(value["output"]["grain"].get("image_sigma").is_some());
    let moved = directory.path().join("elsewhere.antitone");
    restored.save(&moved, Save::New).unwrap();
    std::fs::remove_file(original).unwrap();
    assert_eq!(Project::load(&moved).unwrap(), project);
}

#[test]
fn malformed_embedded_assets_in_either_slot_are_rejected_at_their_actual_fields() {
    let mut project = Project::default();
    project.output.lut = Some(lut(LookOrigin::Imported));
    project.output.film_and_paper = Some(material());
    let value = serde_json::to_value(project).unwrap();
    assert!(
        serde_json::from_value::<Project>(value.clone())
            .unwrap()
            .validate()
            .is_ok()
    );
    for slot in ["lut", "film_and_paper"] {
        for (field, new) in [
            ("contract", serde_json::json!("rec709-drt")),
            ("content_id", serde_json::json!("bad")),
            ("size", serde_json::json!(257)),
        ] {
            let mut bad = value.clone();
            bad["output"][slot][field] = new;
            assert!(
                serde_json::from_value::<Project>(bad).is_err(),
                "{slot}/{field}"
            );
        }
    }
}

#[test]
fn material_and_imported_asset_roles_cannot_be_silently_swapped() {
    let mut project = Project::default();
    project.output.lut = Some(material());
    assert!(project.validate().is_err());
    project.output.lut = None;
    project.output.film_and_paper = Some(lut(LookOrigin::Imported));
    assert!(project.validate().is_err());
    project.output.film_and_paper = Some(material());
    assert!(project.validate().is_ok());
}
