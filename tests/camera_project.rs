#![cfg(feature = "project")]
use antitone::{
    processing::{CAMERA_INPUT_FLOOR, DEFAULT_GAMMA, InputSettings, Response},
    project::{Development, Project, Save, Source, SourceDevelopment, SourceId},
};
use glam::{DMat3, DVec3};

fn source(model: &str, matrix: DMat3) -> Source {
    Source {
        id: SourceId(0),
        path: "test.ARW".into(),
        development: SourceDevelopment {
            decode: Development::new("test-backend".into(), [1.; 4]),
            lens_correction: None,
            dng_development: None,
            dimensions: [900, 600],
            active_area: [0, 0, 900, 600],
            camera_model: model.into(),
            camera_to_ap1: matrix.to_cols_array_2d(),
            input_floor: CAMERA_INPUT_FLOOR,
        },
        reference_only: false,
        adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
            Default::default(),
        )),
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    }
}

fn project() -> Project {
    let mut project = Project {
        input: InputSettings::Negative {
            dmin: Some(DVec3::splat(0.2)),
            dmax: Some(DVec3::splat(0.01)),
            input_floor: CAMERA_INPUT_FLOOR,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Default::default(),
        },
        ..Default::default()
    };
    project
        .add_source(source("camera A", DMat3::IDENTITY))
        .unwrap();
    project
}

#[test]
fn saved_gamma_is_explicit_and_preserved_independently_of_the_new_grade_default() {
    assert_eq!(
        match Project::default().input {
            InputSettings::Negative { gamma, .. } => gamma,
            InputSettings::Digital => DEFAULT_GAMMA,
        },
        DEFAULT_GAMMA
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roll.antitone.json");
    for response in [
        Response::film(),
        Response {
            toe: 3.25,
            shoulder: 0.004,
        },
    ] {
        let mut p = project();
        if let InputSettings::Negative {
            gamma,
            response: selected,
            ..
        } = &mut p.input
        {
            *gamma = 0.65;
            *selected = response;
        }
        let positive = |p: &Project| {
            antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                antitone::adjustment_layers::AdjustmentLayer::global(
                    p.sources[0].adjustment_layers.as_ref().unwrap().layers[0]
                        .adjustment
                        .photo,
                ),
            ])
            .and_then(|layers| {
                antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                    p.input,
                    p.roll,
                    layers,
                    DMat3::IDENTITY,
                )
            })
            .unwrap()
            .apply(DVec3::new(0.02, 0.05, 0.08), [0, 0])
            .unwrap()
            .rgb
        };
        let before = positive(&p);
        p.save(&path, Save::Replace).unwrap();
        let restored = Project::load(&path).unwrap();
        assert_eq!(restored.input, p.input);
        assert_eq!(restored, p);
        assert_eq!(positive(&restored), before);
        let mut missing = serde_json::to_value(&p).unwrap();
        missing["input"].as_object_mut().unwrap().remove("gamma");
        assert!(serde_json::from_value::<Project>(missing).is_err());
    }
}

#[test]
fn analysis_area_roundtrips_without_changing_processing_and_old_documents_default_to_none() {
    let mut p = project();
    let old = serde_json::to_value(&p).unwrap();
    assert!(old.get("analysis_area").is_none());
    assert_eq!(
        serde_json::from_value::<Project>(old)
            .unwrap()
            .analysis_area,
        None
    );
    let before = p.clone();
    p.analysis_area = Some([40, 30, 820, 540]);
    p.validate().unwrap();
    let saved = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<Project>(&saved).unwrap(), p);
    assert_eq!(p.roll, before.roll);
    assert_eq!(p.sources, before.sources);
    assert_eq!(p.calibration().unwrap(), before.calibration().unwrap());
    for area in [[40, 30, 7, 540], [0, 0, 901, 600], [u32::MAX, 0, 8, 8]] {
        p.analysis_area = Some(area);
        assert!(p.validate().is_err());
    }
}

#[test]
fn roll_wb_roundtrips_defaults_missing_optional_values_to_zero_and_rejects_invalid_values() {
    use antitone::color::Balance;
    let mut p = project();
    let mut old = serde_json::to_value(&p).unwrap();
    old["roll"].as_object_mut().unwrap().remove("wb");
    assert_eq!(serde_json::from_value::<Project>(old).unwrap(), p);
    p.roll.wb = Balance {
        warmth: -0.25,
        tint: 0.15,
    };
    p.validate().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roll.antitone.json");
    p.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), p);
    for value in [f64::NAN, f64::INFINITY, 1.01, -1.01] {
        for tint in [false, true] {
            let mut invalid = p.clone();
            if tint {
                invalid.roll.wb.tint = value;
            } else {
                invalid.roll.wb.warmth = value;
            }
            assert!(invalid.validate().is_err());
            assert!(invalid.save(&path, Save::Replace).is_err());
            assert_eq!(Project::load(&path).unwrap(), p);
        }
    }
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
                .require_scan_source(&incompatible.development)
                .is_err()
        );
        assert!(project.add_source(incompatible.clone()).is_err());
        assert_eq!(project, before);
        let mut invalid = project.clone();
        let mut second = incompatible.clone();
        second.id = SourceId(2);
        invalid.sources.push(second);
        invalid.order.push(SourceId(2));
        invalid.next_source_id = 3;
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn camera_basis_survives_source_removal_roundtrip_and_rejects_later_camera_substitution() {
    let mut project = project();
    project.sources.clear();
    project.order.clear();
    project.scan = None;
    project.validate().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    let mut restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    // Clearing the explicit acquisition setup permits a new camera basis.
    restored
        .add_source(source("camera B", DMat3::IDENTITY))
        .unwrap();
    assert!(restored.validate().is_ok());
    assert!(
        restored
            .add_source(source(
                "camera A",
                DMat3::from_diagonal(DVec3::new(1.1, 1., 1.))
            ))
            .is_err()
    );
    // A corrupted document that bypasses admission is still rejected by validation.
    let mut mixed = restored.clone();
    let mut other = source("camera A", DMat3::from_diagonal(DVec3::new(1.1, 1., 1.)));
    other.id = SourceId(2);
    mixed.sources.push(other);
    mixed.order.push(SourceId(2));
    mixed.next_source_id = 3;
    assert!(mixed.validate().is_err());
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
