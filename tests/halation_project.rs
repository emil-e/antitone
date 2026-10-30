#![cfg(feature = "project")]
use antitone::{
    halation::{CONTRACT, HalationSettings},
    project::{Project, Save},
};

#[test]
fn absent_settings_keep_old_output_and_custom_disabled_values_survive_save_open() {
    let legacy = serde_json::to_value(Project::default()).unwrap();
    assert!(legacy["output"].get("halation").is_none());
    let restored: Project = serde_json::from_value(legacy).unwrap();
    assert!(!restored.output.halation.enabled);
    assert!(
        restored
            .output
            .halation
            .parameters([4000, 6000])
            .unwrap()
            .is_none()
    );
    let directory = tempfile::tempdir().unwrap();
    for (enabled, size) in [false, true]
        .into_iter()
        .flat_map(|enabled| [0.103, 0.4, 0.5].map(|size| (enabled, size)))
    {
        let mut project = Project::default();
        project.output.halation = HalationSettings {
            enabled,
            size,
            strength: 0.37,
        };
        assert!(project.input.is_negative());
        assert!(!project.output.film_emulation.enabled);
        project.validate().unwrap();
        let path = directory
            .path()
            .join(format!("halo-{enabled}-{size}.antitone"));
        project.save(&path, Save::New).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
    }
}

#[test]
fn malformed_or_unknown_contract_rejects_even_when_disabled() {
    let mut project = Project::default();
    project.output.halation = HalationSettings {
        enabled: true,
        ..Default::default()
    };
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["output"]["halation"]["contract"], CONTRACT);
    for (key, invalid) in [
        ("contract", serde_json::json!("future-halo-v99")),
        ("extra", serde_json::json!(1)),
        ("size", serde_json::json!(0.)),
        ("strength", serde_json::json!(-0.1)),
        ("strength", serde_json::json!(1.1)),
        ("strength", serde_json::Value::Null),
    ] {
        let mut bad = value.clone();
        bad["output"]["halation"]["enabled"] = false.into();
        bad["output"]["halation"][key] = invalid;
        assert!(serde_json::from_value::<Project>(bad).is_err(), "{key}");
    }
    for key in ["contract", "enabled", "size", "strength"] {
        let mut bad = value.clone();
        bad["output"]["halation"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(
            serde_json::from_value::<Project>(bad).is_err(),
            "missing {key}"
        );
    }
    project.output.halation.enabled = false;
    project.output.halation.size = f64::NAN;
    assert!(project.validate().is_err());
}
