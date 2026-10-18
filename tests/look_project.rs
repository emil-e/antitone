#![cfg(feature = "project")]
use antitone::{
    looks::{LookAsset, LookOrigin, Lut3D},
    project::{Project, Save},
};
use std::sync::Arc;
fn look() -> Arc<LookAsset> {
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
            "Embedded custom look".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    )
}
#[test]
fn old_projects_default_to_no_look_and_embedded_samples_survive_save_as() {
    let mut project = Project::default();
    let old = serde_json::to_value(&project).unwrap();
    assert!(old["output"].get("look").is_none());
    assert!(
        serde_json::from_value::<Project>(old)
            .unwrap()
            .output
            .look
            .is_none()
    );
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.antitone.json");
    project.output.look = Some(look());
    project.save(&original, Save::New).unwrap();
    let restored = Project::load(&original).unwrap();
    assert_eq!(restored, project);
    let elsewhere = directory.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let moved = elsewhere.join("saved.antitone.json");
    restored.save(&moved, Save::New).unwrap();
    std::fs::remove_file(original).unwrap();
    assert_eq!(
        Project::load(&moved).unwrap().output.look,
        project.output.look
    );
}

#[test]
fn historic_builtin_keeps_its_embedded_grid_without_a_current_generator() {
    let legacy = Arc::new(
        LookAsset::new(
            "Historic built-in bake".into(),
            LookOrigin::Builtin {
                id: "kodak-2383".into(),
                version: 1,
                generator: "retired-generator".into(),
                definition: "0".repeat(64),
            },
            Arc::new(Lut3D::new(65, [-0.5; 3], [1.; 3], vec![[0.4; 3]; 65usize.pow(3)]).unwrap()),
        )
        .unwrap(),
    );
    let project = Project {
        output: antitone::project::Output {
            look: Some(legacy.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("historic.antitone");
    project.save(&path, Save::New).unwrap();

    let reopened = Project::load(&path).unwrap().output.look.unwrap();
    assert_eq!(reopened, legacy);
}

#[test]
fn unknown_or_corrupt_embedded_lut_contract_is_not_silently_dropped() {
    let mut project = Project::default();
    project.output.look = Some(look());
    let value = serde_json::to_value(project).unwrap();
    for (field, new) in [
        ("contract", serde_json::json!("rec709-drt")),
        ("content_id", serde_json::json!("bad")),
        ("size", serde_json::json!(257)),
    ] {
        let mut bad = value.clone();
        bad["output"]["look"][field] = new;
        assert!(serde_json::from_value::<Project>(bad).is_err());
    }
}
