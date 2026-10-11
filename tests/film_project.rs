#![cfg(feature = "project")]
use antitone::{
    processing::Response,
    project::{Development, Project, Save},
};
fn project() -> Project {
    Project {
        development: Some(Development::new(
            "0.22.0".into(),
            [2133., 1024., 1771., 1024.],
        )),
        camera_basis: Some(antitone::project::CameraBasis {
            camera_model: "test-camera".into(),
            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
        }),
        dmin: Some([0.1685663501546697, 0.13359162596075455, 0.11715123642832656]),
        dmax: Some([0.008061, 0.003409, 0.001442]),
        input_floor: Some(1. / 65535.),
        ..Project::default()
    }
}
#[test]
fn ap0_project_schemas_and_models_reject_without_reinterpreting_calibration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.antitone.json");
    for model in [2, 3] {
        let mut value = serde_json::to_value(project()).unwrap();
        value["schema"] = 3.into();
        value["processing_model"] = model.into();
        value["development"]["inversion_space"] = "AP0/D60".into();
        if model == 2 {
            value["roll"].as_object_mut().unwrap().remove("response");
        }
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Project::load(&path).is_err());
        let mut incompatible = project();
        incompatible.processing_model = model;
        assert!(incompatible.validate().is_err());
    }
}
#[test]
fn camera_model_roundtrips_explicit_response_and_rejects_missing_or_unknown_shape() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("film.antitone.json");
    let mut film = project();
    assert_eq!(film.processing_model, 4);
    assert_eq!(film.roll.response, Response::film());
    for response in [
        Response::Original,
        Response::Film {
            toe: 2.543210987654321,
            shoulder: 0.003210987654321,
        },
    ] {
        film.roll.response = response;
        film.save(&path, Save::Replace).unwrap();
        assert_eq!(Project::load(&path).unwrap(), film);
        let mut missing = serde_json::to_value(&film).unwrap();
        missing["roll"].as_object_mut().unwrap().remove("response");
        assert!(serde_json::from_value::<Project>(missing).is_err());
        let mut unknown = serde_json::to_value(&film).unwrap();
        unknown["roll"]["response"]["kind"] = "unknown".into();
        assert!(serde_json::from_value::<Project>(unknown).is_err());
        if matches!(response, Response::Film { .. }) {
            let mut unknown = serde_json::to_value(&film).unwrap();
            unknown["roll"]["response"]["unexpected"] = true.into();
            assert!(serde_json::from_value::<Project>(unknown).is_err());
        }
    }
}
#[test]
fn project_validation_rejects_shape_and_calibrated_shoulder_pole() {
    let mut p = project();
    p.roll.response = Response::Film {
        toe: 4.1,
        shoulder: 0.,
    };
    assert!(p.validate().is_err());
    p.roll.response = Response::Film {
        toe: 2.5,
        shoulder: 0.02,
    };
    p.roll.gamma = 0.1;
    p.roll.balance = antitone::color::Balance {
        warmth: -1.,
        tint: 1.,
    };
    assert!(p.validate().is_err());
    p.roll.gamma = 0.65;
    p.roll.balance = antitone::color::Balance::default();
    p.validate().unwrap();
}
