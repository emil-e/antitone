#![cfg(feature = "project")]
use antitone::{
    processing::{PhotoSettings, Pipeline, Response, RollSettings},
    project::{Development, Project, Save},
};
use glam::DVec3;
fn project() -> Project {
    Project {
        development: Some(Development::new(
            "0.22.0".into(),
            [2133., 1024., 1771., 1024.],
        )),
        dmin: Some([0.1685663501546697, 0.13359162596075455, 0.11715123642832656]),
        dmax: Some([0.008061, 0.003409, 0.001442]),
        input_floor: Some(2.1630933622265155e-5),
        ..Project::default()
    }
}
#[test]
fn schema_three_missing_response_keeps_original_model_and_exact_grade() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.antitone.json");
    let mut value = serde_json::to_value(project()).unwrap();
    value["processing_model"] = 2.into();
    value["roll"].as_object_mut().unwrap().remove("response");
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let old = Project::load(&path).unwrap();
    assert_eq!(old.schema, 3);
    assert_eq!(old.processing_model, 2);
    assert_eq!(old.roll, RollSettings::default());
    assert_eq!(serde_json::to_value(&old).unwrap(), value);
    let cal = old.calibration().unwrap().unwrap();
    let pipeline = Pipeline::new(cal, old.roll, PhotoSettings::default()).unwrap();
    let original = Pipeline::new(cal, RollSettings::default(), PhotoSettings::default()).unwrap();
    for scan in [
        cal.dmin,
        cal.dmax,
        cal.dmin * (1. + 1e-12),
        DVec3::new(0.08, 0.04, 0.015),
        DVec3::ZERO,
    ] {
        assert_eq!(pipeline.apply(scan).unwrap(), original.apply(scan).unwrap());
    }
    old.save(&path, Save::Replace).unwrap();
    assert_eq!(Project::load(&path).unwrap(), old);
}
#[test]
fn film_parameters_roundtrip_exactly_and_model_response_mismatch_never_migrates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("film.antitone.json");
    let mut film = project();
    assert_eq!(film.processing_model, 3);
    assert_eq!(film.roll.response, Response::film());
    film.roll.response = Response::Film {
        toe: 2.543210987654321,
        shoulder: 0.003210987654321,
    };
    film.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), film);
    let mut bad = film.clone();
    bad.processing_model = 2;
    assert!(bad.validate().is_err());
    bad = film.clone();
    bad.roll.response = Response::Original;
    assert!(bad.validate().is_err());
    let mut missing = serde_json::to_value(&film).unwrap();
    missing["roll"].as_object_mut().unwrap().remove("response");
    let missing: Project = serde_json::from_value(missing).unwrap();
    assert!(missing.validate().is_err());
    let mut original = film.clone();
    original.roll.response = Response::Original;
    original.processing_model = 2;
    original.validate().unwrap();
    original.roll.response = Response::film();
    assert!(original.validate().is_err());
    original.processing_model = 3;
    original.validate().unwrap();
    assert_eq!(original.dmin, film.dmin);
    assert_eq!(original.dmax, film.dmax);
    assert_eq!(original.development, film.development);
    let mut unknown = serde_json::to_value(&film).unwrap();
    unknown["roll"]["response"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<Project>(unknown).is_err());
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
