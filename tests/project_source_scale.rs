#![cfg(feature = "project")]

use antitone::{processing::InputSettings, project::*};

fn digital(gain: f64) -> Project {
    let mut project = Project {
        input: InputSettings::Digital,
        ..Default::default()
    };
    project
        .add_source(Source {
            id: SourceId(0),
            path: "source.arw".into(),
            reference_only: false,
            development: SourceDevelopment {
                decode: Development::new("test-backend".into(), [gain, 1., 1., 1.]),
                dimensions: [64; 2],
                active_area: [0, 0, 64, 64],
                camera_model: "authored".into(),
                camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            },
            adjustment_layers: Some(AdjustmentLayers::with_global(
                antitone::adjustment_layers::AdjustmentLayerSettings {
                    exposure: 1.25,
                    ..Default::default()
                },
            )),
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    project.roll.exposure = -0.5;
    project
}

#[test]
fn accepted_wb_roundtrips_without_capture_metadata_or_grade_changes() {
    for gain in [1., 2., 3.5, 4.] {
        let project = digital(gain);
        project.require_runtime().unwrap();
        let json = serde_json::to_string(&project).unwrap();
        assert!(!json.contains("pre_applied_wb"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap()["sources"][0]["development"]
                ["decode"]["white_balance"],
            serde_json::json!([gain, 1., 1., 1.])
        );
        let loaded: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, project);
        loaded.require_runtime().unwrap();
        assert!(!json.contains("precision_floor"));
        assert_eq!(loaded.roll.exposure, -0.5);
        assert_eq!(
            loaded.sources[0].grade_snapshot().unwrap().layers()[0]
                .photo
                .exposure,
            1.25
        );
    }
}

#[test]
fn accepted_wb_requires_representable_gains_and_the_selected_backend_basis() {
    let project = digital(1.);
    for value in [
        0.,
        -1.,
        f64::NAN,
        f64::INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
    ] {
        let mut invalid = project.clone();
        invalid.sources[0].development.decode.white_balance[0] = value;
        assert!(invalid.validate().is_err(), "gain {value}");
    }
    let mut sdk = project.clone();
    sdk.sources[0].development.decode.backend = Developer::DngSdk;
    sdk.sources[0].development.decode.white_balance = [4., 1., 2., 1.];
    sdk.validate().unwrap();
    for wb in [[4., 1., 2., 2.], [8., 2., 4., 2.], [257., 1., 2., 1.]] {
        let mut invalid = sdk.clone();
        invalid.sources[0].development.decode.white_balance = wb;
        invalid.validate().unwrap();
    }
    let mut negative = project;
    negative.input = InputSettings::default();
    negative.sources[0].development.decode.white_balance = [2., 1., 1., 1.];
    assert!(
        negative.validate().is_err(),
        "Negative requires unity development"
    );
}
