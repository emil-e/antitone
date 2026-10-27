use super::*;
use crate::{
    exposure_reference::{ReferenceExposure, StartingExposure},
    processing::{CAMERA_INPUT_FLOOR, InputSettings},
    project::{Development, Source},
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
            development,
            revision: FileRevision::read(path).unwrap(),
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
fn exposure_metadata_is_bound_to_the_ordered_source_interpretation() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("source.raw");
    std::fs::write(&file, b"source").unwrap();
    let (project, metadata) = fixture(&file);
    assert_eq!(metadata.for_project(&project), metadata);
    let mut changed = project.clone();
    changed.sources[0].development.camera_model = "other basis".into();
    assert!(metadata.for_project(&changed).entries.is_empty());
    let mut negative = project;
    negative.input = InputSettings::default();
    assert!(metadata.for_project(&negative).entries.is_empty());
}

#[cfg(feature = "raw")]
#[test]
fn restoring_exposure_metadata_checks_current_file_revision_before_populating_the_memo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.raw");
    std::fs::write(&path, b"first").unwrap();
    let (project, metadata) = fixture(&path);
    let store = crate::source_store::SourceStore::new(0);
    store.restore_exposure_metadata(&metadata, &project, |_| Ok(path.clone()));
    let key =
        crate::source_store::SourceKey::capture(&path, &project.sources[0].development).unwrap();
    assert_eq!(
        store.exposure_memo(&key).get(),
        Some(&metadata.entries[0].reference)
    );
    std::fs::write(&path, b"a changed capture").unwrap();
    let next = crate::source_store::SourceStore::new(0);
    next.restore_exposure_metadata(&metadata, &project, |_| Ok(path.clone()));
    let key =
        crate::source_store::SourceKey::capture(&path, &project.sources[0].development).unwrap();
    assert!(next.exposure_memo(&key).get().is_none());
}
