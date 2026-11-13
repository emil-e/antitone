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
fn current_material_labels_do_not_rebake_a_saved_129_cube_with_a_retired_generator() {
    use antitone::looks::{authoring, monochrome};
    use std::sync::atomic::AtomicBool;

    // Deliberately authored numeric data, not a golden result of the current
    // generator: the embedded payload is authoritative even for known labels.
    let mut nodes = Vec::with_capacity(129_usize.pow(3));
    for b in 0..129 {
        for g in 0..129 {
            for r in 0..129 {
                nodes.push([r, g, b].map(|v| 0.125 + v as f32 / 256.));
            }
        }
    }
    let paper = monochrome::Paper::default();
    let negative = monochrome::Negative::from_id("ilford_hp5_plus_ilfotec_hc_6m30s").unwrap();
    let selection = monochrome::Selection {
        negative: Some(negative),
        paper,
    };
    let mut origin = selection.origin();
    let LookOrigin::Photochemical { generator, .. } = &mut origin else {
        unreachable!()
    };
    *generator = "retired-generator".into();
    let saved = Arc::new(
        LookAsset::new(
            selection.name(),
            origin,
            Arc::new(Lut3D::new(129, [-0.5; 3], [1.0386752; 3], nodes).unwrap()),
        )
        .unwrap(),
    );
    let fresh = monochrome::Model::new(selection)
        .bake(authoring::DEFAULT_GRID, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(fresh.lut().size(), 65);
    assert_ne!(fresh.id(), saved.id());
    let mut project = Project::default();
    project.output.film_and_paper = Some(saved.clone());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.antitone");
    project.save(&path, Save::New).unwrap();
    let restored = Project::load(&path).unwrap().output.film_and_paper.unwrap();
    assert_eq!(restored, saved);
    assert_eq!(restored.lut().size(), 129);
    assert_eq!(restored.lut().nodes(), saved.lut().nodes());
    assert_eq!(restored.lut().domain(), saved.lut().domain());
}

#[test]
fn saved_supra_samples_remain_authoritative_when_the_current_paper_generator_changes() {
    use antitone::looks::{authoring, spektrafilm};
    use std::sync::atomic::AtomicBool;
    let saved: Arc<LookAsset> = Arc::new(
        serde_json::from_str(include_str!("fixtures/spektrafilm/supra-original.json")).unwrap(),
    );
    let selection = spektrafilm::Selection {
        negative: None,
        paper: spektrafilm::Paper::from_id("kodak_supra_endura").unwrap(),
    };
    assert_eq!(
        spektrafilm::Selection::from_origin(saved.origin()),
        Some(selection)
    );
    let fresh = spektrafilm::Model::new(selection, &AtomicBool::new(false))
        .unwrap()
        .bake(authoring::DEFAULT_GRID, &AtomicBool::new(false))
        .unwrap();
    assert_ne!(fresh.origin(), saved.origin());
    assert_ne!(fresh.id(), saved.id());
    let mut project = Project::default();
    project.output.film_and_paper = Some(saved.clone());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted-supra.antitone");
    project.save(&path, Save::New).unwrap();
    let reopened = Project::load(&path).unwrap().output.film_and_paper.unwrap();
    assert_eq!(reopened.origin(), saved.origin());
    assert_eq!(reopened.id(), saved.id());
    assert_eq!(reopened.lut().nodes(), saved.lut().nodes());
    assert_eq!(reopened.lut().domain(), saved.lut().domain());
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
