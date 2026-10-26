#![cfg(feature = "project")]
use antitone::{
    color::Balance,
    processing::{DEFAULT_GAMMA, InputSettings, Response},
    project::{Project, Save},
};
use glam::DVec3;

fn project() -> Project {
    Project {
        input: InputSettings::Negative {
            dmin: Some(DVec3::new(
                0.1685663501546697,
                0.13359162596075455,
                0.11715123642832656,
            )),
            dmax: Some(DVec3::new(0.008061, 0.003409, 0.001442)),
            input_floor: 1. / 65535.,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        },
        ..Project::default()
    }
}

fn negative(project: &Project) -> &InputSettings {
    assert!(project.input.is_negative());
    &project.input
}

#[test]
fn ap0_project_schemas_and_models_reject_without_reinterpreting_calibration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.antitone.json");
    for model in [2, 3] {
        let mut value = serde_json::to_value(project()).unwrap();
        value["schema"] = 3.into();
        value["processing_model"] = model.into();
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
    assert_eq!(film.processing_model, antitone::PROCESSING_MODEL_VERSION);
    assert_eq!(
        match negative(&film) {
            InputSettings::Negative { response, .. } => *response,
            InputSettings::Digital => unreachable!(),
        },
        Response::film()
    );
    for response in [
        Response::film(),
        Response {
            toe: 2.543210987654321,
            shoulder: 0.003210987654321,
        },
    ] {
        if let InputSettings::Negative {
            response: selected, ..
        } = &mut film.input
        {
            *selected = response;
        }
        film.save(&path, Save::Replace).unwrap();
        assert_eq!(Project::load(&path).unwrap(), film);
        let mut missing = serde_json::to_value(&film).unwrap();
        missing["input"].as_object_mut().unwrap().remove("response");
        assert!(serde_json::from_value::<Project>(missing).is_err());
        let mut unknown = serde_json::to_value(&film).unwrap();
        unknown["input"]["response"]["kind"] = "unknown".into();
        assert!(serde_json::from_value::<Project>(unknown).is_err());
        let mut unknown = serde_json::to_value(&film).unwrap();
        unknown["input"]["response"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<Project>(unknown).is_err());
    }
}

#[test]
fn project_validation_rejects_shape_and_calibrated_shoulder_pole() {
    let mut p = project();
    if let InputSettings::Negative {
        response,
        gamma,
        balance,
        ..
    } = &mut p.input
    {
        *response = Response {
            toe: 4.1,
            shoulder: 0.,
        };
        *gamma = DEFAULT_GAMMA;
        *balance = Balance::default();
    }
    assert!(p.validate().is_err());
    if let InputSettings::Negative {
        response,
        gamma,
        balance,
        ..
    } = &mut p.input
    {
        *response = Response {
            toe: 2.5,
            shoulder: 0.02,
        };
        *gamma = 0.1;
        *balance = Balance {
            warmth: -1.,
            tint: 1.,
        };
    }
    assert!(p.validate().is_err());
    if let InputSettings::Negative { gamma, balance, .. } = &mut p.input {
        *gamma = 0.65;
        *balance = Balance::default();
    }
    p.validate().unwrap();
}
