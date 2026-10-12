#![cfg(feature = "project")]
use antitone::{
    processing::CAMERA_INPUT_FLOOR,
    project::{Development, Fingerprint, Project, Save, Source, SourceDevelopment, SourceId},
};
use glam::{DMat3, DVec3};

fn source(model: &str, matrix: DMat3) -> Source {
    Source {
        id: SourceId(0),
        path: "test.ARW".into(),
        fingerprint: Fingerprint {
            bytes: 1,
            blake3: "a".repeat(64),
        },
        development: SourceDevelopment {
            dimensions: [900, 600],
            active_area: [0, 0, 900, 600],
            camera_model: model.into(),
            camera_to_ap1: matrix.to_cols_array_2d(),
            input_floor: CAMERA_INPUT_FLOOR,
        },
        reference_only: false,
        photo: Default::default(),
        orientation: Default::default(),
        crop: None,
    }
}
fn project() -> Project {
    let mut project = Project {
        development: Some(Development::new("test-backend".into(), [2., 1., 1.7, 1.])),
        dmin: Some([0.2; 3]),
        dmax: Some([0.01; 3]),
        input_floor: Some(CAMERA_INPUT_FLOOR),
        ..Default::default()
    };
    project
        .add_source(source("camera A", DMat3::IDENTITY))
        .unwrap();
    project
}

#[test]
fn incompatible_native_camera_sources_and_reference_picks_reject_without_mutation() {
    let mut project = project();
    let before = project.clone();
    for incompatible in [
        source("camera B", DMat3::IDENTITY),
        source("camera A", DMat3::from_diagonal(DVec3::new(1.1, 1., 1.))),
    ] {
        assert!(
            project
                .require_camera_basis(&incompatible.development)
                .is_err()
        );
        assert!(
            project
                .camera_basis
                .as_ref()
                .unwrap()
                .require_source(&incompatible.development)
                .is_err()
        );
        assert!(project.add_source(incompatible.clone()).is_err());
        assert_eq!(project, before);
        let mut invalid = project.clone();
        invalid.sources[0].development = incompatible.development;
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn camera_basis_survives_source_removal_roundtrip_and_rejects_later_camera_substitution() {
    let mut project = project();
    project.sources.clear();
    project.order.clear();
    project.validate().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    let mut restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert!(
        restored
            .add_source(source("camera B", DMat3::IDENTITY))
            .is_err()
    );
    restored.camera_basis = None;
    assert!(restored.validate().is_err());
}

#[test]
fn altered_roll_or_source_base_floor_is_not_a_valid_camera_model_project() {
    let project = project();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.antitone.json");
    for floor in [0., CAMERA_INPUT_FLOOR * 0.5, CAMERA_INPUT_FLOOR * 2.] {
        for source_floor in [false, true] {
            let mut value = serde_json::to_value(&project).unwrap();
            if source_floor {
                value["sources"][0]["development"]["input_floor"] = floor.into();
            } else {
                value["input_floor"] = floor.into();
            }
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(Project::load(&path).is_err());
        }
    }
}
