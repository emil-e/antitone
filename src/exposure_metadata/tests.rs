use super::*;
use crate::{
    exposure_reference::{ReferenceExposure, StartingExposure},
    processing::{CAMERA_INPUT_FLOOR, InputSettings},
    project::{Development, Source, SourceDevelopment},
};

fn fixture(path: &std::path::Path) -> (Project, Metadata) {
    let development = SourceDevelopment {
        decode: Development::new("declared".into(), [1.; 4]),
        lens_correction: None,
        dng_development: None,
        dimensions: [64; 2],
        active_area: [0, 0, 64, 64],
        camera_model: "declared".into(),
        camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
        input_floor: CAMERA_INPUT_FLOOR,
    };
    let mut project = Project {
        input: InputSettings::Digital,
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: path.to_owned(),
            reference_only: false,
            development: development.clone(),
            adjustment_layers: None,
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    let metadata = Metadata {
        version: VERSION,
        entries: vec![Entry {
            source: id,

            reference: ReferenceExposure::Matched(StartingExposure {
                ev: 1.3,
                samples: 1024,
                spread: 0.2,
            }),
        }],
    };
    (project, metadata)
}

#[test]
fn optional_invalid_exposure_metadata_is_discarded_without_reinterpreting_accepted_controls() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("source.raw");
    std::fs::write(&file, b"source").unwrap();
    let (project, metadata) = fixture(&file);
    let mut saved = project.clone();
    saved.exposure_metadata = Some(metadata);
    let value = serde_json::to_value(&saved).unwrap();
    let valid: Project = serde_json::from_value(value.clone()).unwrap();
    assert!(valid.exposure_metadata.is_some());
    for failure in ["version", "gain", "samples", "unknown"] {
        let mut changed = value.clone();
        let cache = &mut changed["exposure_metadata"];
        match failure {
            "version" => cache["version"] = serde_json::json!(VERSION + 1),
            "gain" => cache["entries"][0]["reference"]["Matched"]["ev"] = serde_json::json!(1000.),
            "samples" => {
                cache["entries"][0]["reference"]["Matched"]["samples"] = serde_json::json!(0)
            }
            "unknown" => {
                cache["unexpected"] = serde_json::json!(true);
            }
            _ => unreachable!(),
        }
        let loaded: Project = serde_json::from_value(changed).unwrap();
        assert!(loaded.exposure_metadata.is_none(), "{failure}");
        assert_eq!(loaded, project);
    }
}

#[test]
fn exposure_baseline_follows_the_photo_across_development_and_input_changes() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("source.raw");
    std::fs::write(&file, b"source").unwrap();
    let (project, metadata) = fixture(&file);
    assert_eq!(metadata.for_project(&project), metadata);
    let mut changed = project.clone();
    changed.sources[0].development.camera_model = "other basis".into();
    assert_eq!(metadata.for_project(&changed), metadata);
    let mut negative = project;
    negative.input = InputSettings::default();
    assert_eq!(metadata.for_project(&negative), metadata);
}

#[cfg(feature = "raw")]
#[test]
fn legacy_identity_fields_do_not_discard_a_saved_photo_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.raw");
    std::fs::write(&path, b"first").unwrap();
    let (mut project, metadata) = fixture(&path);
    let mut encoded = serde_json::to_value(&metadata).unwrap();
    encoded["entries"][0]["development"] =
        serde_json::to_value(&project.sources[0].development).unwrap();
    encoded["entries"][0]["revision"] =
        serde_json::to_value(crate::source::FileRevision::read(&path).unwrap()).unwrap();
    std::fs::write(&path, b"a different capture revision").unwrap();
    project.sources[0].development.decode.backend_version = "another decoder".into();
    let restored: Metadata = serde_json::from_value(encoded).unwrap();
    let store = Store::restored(&restored, |_| Some(path.clone()));
    assert_eq!(store.snapshot(&project, |_| Some(path.clone())), metadata);
}

#[cfg(feature = "raw")]
#[test]
fn undo_branch_reusing_a_numeric_photo_id_does_not_inherit_another_photos_initialization() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.raw");
    let second = directory.path().join("second.raw");
    let (project, metadata) = fixture(&first);
    let id = project.sources[0].id;
    let store = Store::restored(&metadata, |_| Some(first.clone()));
    let next = store.memo(id, &second);
    assert!(next.get().is_none());
    assert!(
        store
            .snapshot(&project, |_| Some(second.clone()))
            .entries
            .is_empty()
    );
    assert_eq!(
        store.get(id, &first),
        Some(metadata.entries[0].reference.clone())
    );
}
