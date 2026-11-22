#![cfg(feature = "project")]

use antitone::{
    processing::InputSettings,
    project::{Development, DevelopmentMode, Project, Save, Source, SourceDevelopment, SourceId},
};

fn project() -> Project {
    let mut project = Project::default();
    for path in ["one.dng", "two.dng"] {
        project
            .add_source(Source {
                id: SourceId(0),
                path: path.into(),
                capture_time: None,
                development_mode: DevelopmentMode::Unity,
                development: SourceDevelopment {
                    decode: Development::new("test".into(), [1.; 4]),
                    dimensions: [64, 48],
                    active_area: [0, 0, 64, 48],
                    camera_model: "test camera".into(),
                    camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                },
                repairs: Default::default(),
                adjustment_layers: None,
                orientation: Default::default(),
                crop: None,
            })
            .unwrap();
    }
    project
}

#[test]
fn direct_parsing_establishes_format_integrity_without_a_second_admission_call() {
    let project = project();
    let encoded = serde_json::to_value(&project).unwrap();
    assert_eq!(
        serde_json::from_value::<Project>(encoded.clone()).unwrap(),
        project
    );
    for (pointer, value) in [
        ("/processing_model", serde_json::json!(0)),
        ("/output/contract", serde_json::json!("unsupported output")),
        ("/input/gamma", serde_json::json!(0.)),
        ("/input/response/toe", serde_json::json!(4.1)),
        ("/input/dmin", serde_json::json!([0., 0.1, 0.1])),
        ("/roll/exposure", serde_json::json!(17.)),
        (
            "/sources/0/development/active_area",
            serde_json::json!([63, 0, 8, 8]),
        ),
        ("/sources/0/path", serde_json::json!("")),
        ("/sources/1/id", serde_json::json!(1)),
        ("/next_source_id", serde_json::json!(2)),
        ("/order", serde_json::json!([1])),
        ("/order", serde_json::json!([1, 1])),
    ] {
        let mut malformed = encoded.clone();
        *malformed.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<Project>(malformed).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn adding_a_smaller_photo_retains_the_shared_analysis_prior_and_remains_saveable() {
    let mut project = project();
    let prior = [24, 16, 32, 24];
    project.analysis_area = Some(prior);
    let mut smaller = project.sources[0].clone();
    smaller.path = "smaller.dng".into();
    smaller.development.dimensions = [16, 16];
    smaller.development.active_area = [0, 0, 16, 16];
    let id = project.add_source(smaller).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("larger-roll.antitone");
    project.save(&path, Save::New).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(reopened, project);
    assert_eq!(reopened.analysis_area, Some(prior));
    assert_eq!(
        reopened.source(id).unwrap().development.active_area,
        [0, 0, 16, 16]
    );
}

#[test]
fn malformed_in_memory_records_cannot_be_serialized_or_replace_a_saved_project() {
    let project = project();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.antitone");
    project.save(&path, Save::New).unwrap();
    let accepted = std::fs::read(&path).unwrap();
    for malformed in [
        {
            let mut p = project.clone();
            p.roll.exposure = f64::NAN;
            p
        },
        {
            let mut p = project.clone();
            p.sources[0].path.clear();
            p
        },
        {
            let mut p = project.clone();
            p.order.pop();
            p
        },
        {
            let mut p = project.clone();
            if let InputSettings::Negative { gamma, .. } = &mut p.input {
                *gamma = 0.;
            }
            p
        },
    ] {
        assert!(serde_json::to_value(&malformed).is_err());
        assert!(malformed.save(&path, Save::Replace).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), accepted);
        assert_eq!(Project::load(&path).unwrap(), project);
    }
}
