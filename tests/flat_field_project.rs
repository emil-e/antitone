#![cfg(feature = "project")]
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    flat_field::FlatField,
    processing::{DEFAULT_GAMMA, InputSettings, NegativeInput, Response},
    project::{Development, FlatFieldSettings, Project, Save, Source, SourceDevelopment, SourceId},
};
use glam::{DMat3, DVec3};
use std::sync::Arc;

fn fixture() -> Project {
    let mut project = Project {
        input: InputSettings::Negative {
            dmin: Some(DVec3::splat(0.2)),
            dmax: Some(DVec3::splat(0.001)),
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Default::default(),
        },
        ..Project::default()
    };
    let source = Source {
        id: SourceId(0),
        path: "photo.ARW".into(),
        development: SourceDevelopment {
            decode: Development::new("test-backend".into(), [1.; 4]),
            dimensions: [824, 624],
            active_area: [12, 12, 800, 600],
            camera_model: "test-camera".into(),
            camera_to_ap1: DMat3::IDENTITY.to_cols_array_2d(),
        },
        adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
            AdjustmentLayerSettings::default(),
        )),
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
    project
        .set_flat_field(Some(FlatFieldSettings::new(
            project.source(id).unwrap().path.clone(),
            map,
        )))
        .unwrap();
    project
}

#[test]
fn reference_provenance_stores_only_the_filename() {
    let mut project = fixture();
    let map = project.flat_field().unwrap().map.clone();
    let reference = std::path::Path::new("outside-the-roll").join("light.ARW");
    let settings = FlatFieldSettings::new(reference, map);
    assert_eq!(settings.reference_name, "light.ARW");
    project.set_flat_field(Some(settings)).unwrap();
    let json = serde_json::to_string(&project).unwrap();
    assert!(!json.contains("outside-the-roll"));
    let reopened: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(reopened.flat_field().unwrap().reference_name, "light.ARW");
}

#[test]
fn flat_field_use_does_not_require_matching_libraw_release_provenance() {
    let mut project = fixture();
    let original_map = project.flat_field().unwrap().clone();
    project.sources[0].development.decode.backend_version = "another LibRaw release".into();
    project.validate().unwrap();
    assert_eq!(project.flat_field().unwrap(), &original_map);
}

#[test]
fn map_roundtrip_is_exact_and_does_not_require_reference_file_or_resample_rgb() {
    let mut project = fixture();
    let correction = project.flat_field().cloned();
    let calibration = project.calibration().unwrap();
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
    assert_eq!(reopened.flat_field(), correction.as_ref());
    assert_eq!(reopened.calibration().unwrap(), calibration);
    let mut disabled = project.clone();
    disabled.flat_field_mut().unwrap().enabled = false;
    assert_eq!(disabled.calibration().unwrap(), calibration);
    let mut plain = project;
    plain.set_flat_field(None).unwrap();
    assert_eq!(plain.calibration().unwrap(), calibration);
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("flat_field"));
    assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), plain);
}

#[test]
fn contract_identity_geometry_and_incompatible_sources_reject() {
    let project = fixture();
    let mut changed = project.clone();
    changed.flat_field_mut().unwrap().reference_name = "reference.ARW".into();
    changed.validate().unwrap();
    changed = project.clone();
    changed.sources[0].development.decode.white_balance[0] = 2.;
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.sources[0].development.camera_to_ap1[0][0] += 0.1;
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.flat_field_mut().unwrap().map =
        Arc::new(FlatField::from_nodes([0, 0, 10, 10], [2, 2], vec![DVec3::ONE; 4]).unwrap());
    changed.validate().unwrap();
    changed = project.clone();
    let mut other = changed.sources[0].clone();
    other.development.dimensions = [1024, 824];
    other.path = "other.ARW".into();
    changed.add_source(other).unwrap();
    changed.flat_field_mut().unwrap().enabled = false;
    changed.validate().unwrap();
    let mut json = serde_json::to_value(&project).unwrap();
    json["scan"]["flat_field"]["map"]["size"] = serde_json::json!([0, 2]);
    assert!(serde_json::from_value::<Project>(json).is_err());
}

#[test]
fn dense_calibration_remains_usable_with_or_without_correction() {
    use antitone::processing::{CAMERA_INPUT_FLOOR, Inversion, Response};

    let directory = tempfile::tempdir().unwrap();
    for (index, (anchor_scale, flat_gain)) in [
        (2. * std::f64::consts::SQRT_2, 1.),
        (1., 2. * std::f64::consts::SQRT_2),
        (2. * std::f64::consts::SQRT_2, 1.1),
    ]
    .into_iter()
    .enumerate()
    {
        let mut project = fixture();
        project.set_flat_field(None).unwrap();
        if flat_gain > 1. {
            let map = Arc::new(
                FlatField::from_nodes([30, 30, 760, 550], [2, 2], vec![DVec3::splat(flat_gain); 4])
                    .unwrap(),
            );
            project
                .set_flat_field(Some(FlatFieldSettings::new(
                    project.sources[0].path.clone(),
                    map,
                )))
                .unwrap();
        }

        let floor = CAMERA_INPUT_FLOOR * anchor_scale * flat_gain;
        if let InputSettings::Negative {
            dmin,
            dmax,
            gamma,
            response,
            ..
        } = &mut project.input
        {
            *dmin = Some(DVec3::splat(2.));
            *dmax = Some(DVec3::splat(floor * 1.01));
            *gamma = 0.1;
            *response = Response {
                toe: 1.,
                shoulder: 0.02,
            };
        }
        let negative = project.input.negative_input().unwrap();
        let native = negative.calibration;
        // Dense anchors remain usable without tying the curve domain to correction strength.
        let expected = Inversion::new(negative)
            .unwrap()
            .apply(DVec3::ZERO)
            .unwrap();
        assert!(expected.rgb.is_finite() && expected.rgb.min_element() > 1.);
        Inversion::new(NegativeInput {
            calibration: native,
            ..negative
        })
        .unwrap();
        project.validate().unwrap();

        let path = directory.path().join(format!("floor-{index}.antitone"));
        project.save(&path, Save::New).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);

        if flat_gain == 1. {
            let mut native_source = project.sources[0].clone();
            native_source.path = "native.ARW".into();
            let mut mixed = project.clone();
            mixed.add_source(native_source).unwrap();
            mixed.validate().unwrap();
            assert_eq!(mixed.input, project.input);
            project.sources.clear();
            project.order.clear();
            project.scan = None;
            project.validate().unwrap();
            assert_eq!(
                Inversion::new(project.input.negative_input().unwrap())
                    .unwrap()
                    .apply(DVec3::ZERO)
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn digital_retains_dormant_scan_correction_without_applying_or_enforcing_it() {
    let mut project = fixture();
    project.validate().unwrap();
    let negative = project.input;
    let scan = project.scan.clone();
    assert!(project.active_flat_field().is_some());
    project.input = InputSettings::Digital;
    assert!(project.active_flat_field().is_none());
    assert_eq!(project.scan, scan);
    assert!(
        project
            .image_recipe(project.order[0])
            .unwrap()
            .neutral
            .correction
            .is_none()
    );

    let mut other = project.sources[0].clone();
    other.path = "other-camera.ARW".into();
    other.development.camera_model = "another camera".into();
    let other_id = project.add_source(other).unwrap();
    project.validate().unwrap();
    assert!(
        project
            .image_recipe(other_id)
            .unwrap()
            .neutral
            .correction
            .is_none()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("digital.antitone.json");
    project.save(&path, Save::New).unwrap();
    let mut reopened = Project::load(&path).unwrap();
    assert_eq!(reopened, project);
    assert_eq!(reopened.scan, scan);
    assert!(reopened.require_scan_sources().is_err());

    reopened.sources.retain(|source| source.id != other_id);
    reopened.order.retain(|id| *id != other_id);
    reopened.require_scan_sources().unwrap();
    reopened.input = negative;
    reopened.validate().unwrap();
    assert_eq!(
        reopened.active_flat_field(),
        scan.as_ref().unwrap().flat_field.as_ref()
    );
}

#[test]
fn scan_reentry_can_use_the_saved_map_with_changed_developed_dimensions() {
    let mut project = fixture();
    let negative = project.input;
    project.input = InputSettings::Digital;
    let mut other = project.sources[0].clone();
    other.path = "larger-same-camera.ARW".into();
    other.development.dimensions = [1024, 824];
    assert_eq!(
        antitone::project::ScanBasis::from_development(&other.development),
        antitone::project::ScanBasis::from_development(&project.sources[0].development)
    );
    project.add_source(other).unwrap();
    project.validate().unwrap();
    assert!(project.active_flat_field().is_none());
    project.input = negative;
    project.validate().unwrap();
    assert!(project.active_flat_field().is_some());
    project.flat_field_mut().unwrap().enabled = false;
    project.require_scan_sources().unwrap();
}

#[test]
fn removing_a_photo_deletes_its_record_without_removing_the_map() {
    let mut project = fixture();
    let id = project.order[0];
    project.remove_from_filmstrip(id).unwrap();
    project.validate().unwrap();
    assert!(project.order.is_empty());
    assert!(project.flat_field().is_some());
    assert!(project.source(id).is_err());
}
