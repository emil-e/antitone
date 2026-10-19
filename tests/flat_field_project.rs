#![cfg(feature = "project")]
use antitone::{
    flat_field::FlatField,
    processing::PhotoSettings,
    project::{Development, FlatFieldSettings, Project, Save, Source, SourceDevelopment, SourceId},
};
use glam::{DMat3, DVec3};
use std::sync::Arc;

fn fixture() -> Project {
    let mut project = Project {
        development: Some(Development::new("test-backend".into(), [2., 1., 1.7, 1.])),
        input_floor: Some(1. / 65535.),
        dmin: Some([0.2; 3]),
        dmax: Some([0.001; 3]),
        ..Project::default()
    };
    let source = Source {
        id: SourceId(0),
        path: "photo.ARW".into(),
        development: SourceDevelopment {
            lens_correction: None,
            dng_development: None,
            dimensions: [824, 624],
            active_area: [12, 12, 800, 600],
            camera_model: "test-camera".into(),
            camera_to_ap1: DMat3::IDENTITY.to_cols_array_2d(),
            input_floor: 1. / 65535.,
        },
        reference_only: false,
        photo: PhotoSettings::default(),
        local_adjustments: None,
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    };
    let id = project.add_source(source).unwrap();
    let map = Arc::new(
        FlatField::from_nodes(
            [30, 30, 760, 550],
            [2, 2],
            vec![
                DVec3::new(0.932345678912345, 1.01, 0.98),
                DVec3::new(1.2, 1.3, 1.1),
                DVec3::ONE,
                DVec3::splat(1.1),
            ],
        )
        .unwrap(),
    );
    project.flat_field = Some(FlatFieldSettings::new(
        project.source(id).unwrap(),
        map,
        project.development.as_ref().unwrap(),
    ));
    project
}

#[test]
fn map_roundtrip_is_exact_and_does_not_require_reference_file_or_resample_rgb() {
    let mut project = fixture();
    let correction = project.flat_field.clone();
    project.orientation.turn_clockwise();
    project.orientation.horizontal = true;
    project.sources[0].orientation.turn_counterclockwise();
    project.sources[0].orientation.flips.vertical = true;
    project.sources[0].crop = Some(antitone::crop::Crop {
        center: [412., 312.],
        size: [420, 310],
        angle: 0.27,
    });
    project.validate().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(reopened, project);
    assert_eq!(reopened.flat_field, correction);
    assert_eq!(
        reopened.calibration().unwrap(),
        project.calibration().unwrap()
    );
    assert!(
        reopened
            .processing_calibration()
            .unwrap()
            .unwrap()
            .input_floor
            > project.input_floor.unwrap()
    );
    let mut disabled = project.clone();
    disabled.flat_field.as_mut().unwrap().enabled = false;
    assert_eq!(
        disabled.processing_calibration().unwrap(),
        disabled.calibration().unwrap()
    );
    let mut plain = project;
    plain.flat_field = None;
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("flat_field"));
    assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), plain);
}

#[test]
fn contract_identity_geometry_and_incompatible_sources_reject() {
    let project = fixture();
    let mut changed = project.clone();
    changed.flat_field.as_mut().unwrap().contract = "unknown".into();
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.flat_field.as_mut().unwrap().reference = SourceId(999);
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.development.as_mut().unwrap().fixed_wb[0] += 0.1;
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.sources[0].development.camera_to_ap1[0][0] += 0.1;
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.flat_field.as_mut().unwrap().map =
        Arc::new(FlatField::from_nodes([0, 0, 10, 10], [2, 2], vec![DVec3::ONE; 4]).unwrap());
    assert!(changed.validate().is_err());
    changed = project.clone();
    let mut other = changed.sources[0].clone();
    other.development.dimensions = [1024, 824];
    other.path = "other.ARW".into();
    changed.add_source(other).unwrap();
    assert!(changed.validate().is_err());
    changed.flat_field.as_mut().unwrap().enabled = false;
    changed.validate().unwrap();
    let mut json = serde_json::to_value(&project).unwrap();
    json["flat_field"]["map"]["size"] = serde_json::json!([0, 2]);
    assert!(serde_json::from_value::<Project>(json).is_err());
}

#[test]
fn inversion_domain_uses_each_developed_floor_and_residual_gain_not_the_native_base() {
    use antitone::processing::{CAMERA_INPUT_FLOOR, Calibration, Inversion, Response};

    let directory = tempfile::tempdir().unwrap();
    for (index, (source_gain, flat_gain)) in [
        (2. * std::f64::consts::SQRT_2, 1.),
        (1., 2. * std::f64::consts::SQRT_2),
        (2. * std::f64::consts::SQRT_2, 1.1),
    ]
    .into_iter()
    .enumerate()
    {
        let mut project = fixture();
        project.flat_field = None;
        project.sources[0].development.input_floor = CAMERA_INPUT_FLOOR * source_gain;
        project.sources[0].development.lens_correction = (source_gain > 1.).then(|| "1".repeat(64));
        if flat_gain > 1. {
            let map = Arc::new(
                FlatField::from_nodes([30, 30, 760, 550], [2, 2], vec![DVec3::splat(flat_gain); 4])
                    .unwrap(),
            );
            project.flat_field = Some(FlatFieldSettings::new(
                &project.sources[0],
                map,
                project.development.as_ref().unwrap(),
            ));
        }

        let floor = CAMERA_INPUT_FLOOR * source_gain * flat_gain;
        project.dmin = Some([2.; 3]);
        project.dmax = Some([floor * 1.01; 3]);
        project.roll.gamma = 0.1;
        project.roll.response = Response::Film {
            toe: 1.,
            shoulder: 0.02,
        };
        let native = project.calibration().unwrap().unwrap();
        // The shoulder pole is below the usable developed domain, but above the native floor.
        assert!(Inversion::new(native, project.roll).is_err());
        Inversion::new(
            Calibration {
                input_floor: floor,
                ..native
            },
            project.roll,
        )
        .unwrap();
        project.validate().unwrap();

        let path = directory.path().join(format!("floor-{index}.antitone"));
        project.save(&path, Save::New).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);

        if flat_gain == 1. {
            let mut native_source = project.sources[0].clone();
            native_source.path = "native.ARW".into();
            native_source.development.lens_correction = None;
            native_source.development.input_floor = CAMERA_INPUT_FLOOR;
            let mut mixed = project.clone();
            mixed.add_source(native_source).unwrap();
            assert!(
                mixed.validate().is_err(),
                "every source needs a valid inversion domain"
            );
            project.sources.clear();
            project.order.clear();
            assert!(
                project.validate().is_err(),
                "source-less calibration retains the base domain"
            );
        }
    }
}

#[test]
fn reference_kept_visible_and_removing_it_from_order_does_not_remove_map() {
    let mut project = fixture();
    let id = project.order[0];
    assert!(!project.source(id).unwrap().reference_only);
    project.remove_from_filmstrip(id).unwrap();
    project.validate().unwrap();
    assert!(project.order.is_empty());
    assert!(project.flat_field.is_some());
    assert!(project.source(id).is_ok());
}
