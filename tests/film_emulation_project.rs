#![cfg(feature = "project")]
use antitone::{
    film_emulation::{CONTRACT, FilmEmulationPreset, FilmEmulationSettings},
    project::{Project, Save},
};

#[test]
fn projects_without_film_settings_keep_the_clean_plate_and_custom_disabled_values_survive_save_as()
{
    let legacy = serde_json::to_value(Project::default()).unwrap();
    assert!(legacy["output"].get("film_emulation").is_none());
    let restored: Project = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        restored.output.film_emulation,
        FilmEmulationSettings::default()
    );
    let directory = tempfile::tempdir().unwrap();
    for enabled in [false, true] {
        let mut project = Project::default();
        project.output.film_emulation = FilmEmulationSettings {
            enabled,
            amount: 0.083,
            size: 0.042,
            colour: 0.73,
        };
        // Digital-only visibility must not become a saved/processing-stage restriction.
        assert!(project.input.is_negative());
        project.validate().unwrap();
        let original = directory
            .path()
            .join(format!("original-{enabled}.antitone"));
        project.save(&original, Save::New).unwrap();
        let restored = Project::load(&original).unwrap();
        assert_eq!(restored, project);
        let copied = directory.path().join(format!("copy-{enabled}.antitone"));
        restored.save(&copied, Save::New).unwrap();
        std::fs::remove_file(original).unwrap();
        assert_eq!(Project::load(&copied).unwrap(), project);
    }
}

#[test]
fn unsupported_or_malformed_film_settings_reject_instead_of_silently_changing_appearance() {
    let mut project = Project::default();
    project.output.film_emulation = FilmEmulationPreset::Medium.settings();
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["output"]["film_emulation"]["contract"], CONTRACT);
    for (key, bad_value) in [
        ("contract", serde_json::json!("future-grain-v999")),
        ("extra", serde_json::json!(1)),
        ("size", serde_json::json!(0.)),
        ("amount", serde_json::json!(0.21)),
        ("colour", serde_json::json!(-0.1)),
        ("amount", serde_json::Value::Null),
    ] {
        let mut bad = value.clone();
        bad["output"]["film_emulation"][key] = bad_value;
        assert!(serde_json::from_value::<Project>(bad).is_err(), "{key}");
    }
    for missing in ["contract", "amount", "size", "colour", "enabled"] {
        let mut bad = value.clone();
        bad["output"]["film_emulation"]
            .as_object_mut()
            .unwrap()
            .remove(missing);
        assert!(
            serde_json::from_value::<Project>(bad).is_err(),
            "missing {missing}"
        );
    }
    project.output.film_emulation.enabled = false;
    project.output.film_emulation.amount = f64::NAN;
    assert!(project.validate().is_err());
}
