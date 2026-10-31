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
            input_floor: 1. / 65535.,
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
            lens_correction: None,
            dng_development: None,
            dimensions: [824, 624],
            active_area: [12, 12, 800, 600],
            camera_model: "test-camera".into(),
            camera_to_ap1: DMat3::IDENTITY.to_cols_array_2d(),
            input_floor: 1. / 65535.,
        },
        reference_only: false,
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
    project.set_flat_field(Some(FlatFieldSettings::new(
        project.source(id).unwrap(),
        map,
    )));
    project
}

#[test]
fn flat_field_use_does_not_require_matching_libraw_release_provenance() {
    let mut project = fixture();
    let original_map = project.flat_field().unwrap().clone();
    project.sources[0].development.decode.backend_version = "another LibRaw release".into();
    project.validate().unwrap();
    original_map
        .require_source(&project.sources[0].development)
        .unwrap();
    assert_eq!(project.flat_field().unwrap(), &original_map);
}

#[test]
fn map_roundtrip_is_exact_and_does_not_require_reference_file_or_resample_rgb() {
    let mut project = fixture();
    let correction = project.flat_field().cloned();
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
            > project.calibration().unwrap().unwrap().input_floor
    );
    let mut disabled = project.clone();
    disabled.flat_field_mut().unwrap().enabled = false;
    assert_eq!(
        disabled.processing_calibration().unwrap(),
        disabled.calibration().unwrap()
    );
    let mut plain = project;
    plain.set_flat_field(None);
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("flat_field"));
    assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), plain);
}

#[test]
fn contract_identity_geometry_and_incompatible_sources_reject() {
    let project = fixture();
    let mut changed = project.clone();
    changed.flat_field_mut().unwrap().contract = "unknown".into();
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.flat_field_mut().unwrap().reference = SourceId(999);
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.sources[0].development.decode.fixed_wb[0] += 0.1;
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.sources[0].development.camera_to_ap1[0][0] += 0.1;
    assert!(changed.validate().is_err());
    changed = project.clone();
    changed.flat_field_mut().unwrap().map =
        Arc::new(FlatField::from_nodes([0, 0, 10, 10], [2, 2], vec![DVec3::ONE; 4]).unwrap());
    assert!(changed.validate().is_err());
    changed = project.clone();
    let mut other = changed.sources[0].clone();
    other.development.dimensions = [1024, 824];
    other.path = "other.ARW".into();
    assert!(changed.add_source(other.clone()).is_err());
    changed.flat_field_mut().unwrap().enabled = false;
    changed.add_source(other).unwrap();
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
        project.set_flat_field(None);
        project.sources[0].development.input_floor = CAMERA_INPUT_FLOOR * source_gain;
        project.sources[0].development.lens_correction = (source_gain > 1.).then(|| "1".repeat(64));
        if flat_gain > 1. {
            let map = Arc::new(
                FlatField::from_nodes([30, 30, 760, 550], [2, 2], vec![DVec3::splat(flat_gain); 4])
                    .unwrap(),
            );
            project.set_flat_field(Some(FlatFieldSettings::new(&project.sources[0], map)));
        }

        let floor = CAMERA_INPUT_FLOOR * source_gain * flat_gain;
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
        // The shoulder pole is below the usable developed domain, but above the native floor.
        assert!(Inversion::new(negative).is_err());
        Inversion::new(NegativeInput {
            calibration: Calibration {
                input_floor: floor,
                ..native
            },
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
    other.development.decode.fixed_wb = [1.; 4];
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
fn scan_reentry_checks_dormant_correction_geometry_before_activation() {
    let mut project = fixture();
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
    assert!(
        project
            .require_scan_sources()
            .unwrap_err()
            .to_string()
            .contains("flat-field")
    );
    project.flat_field_mut().unwrap().enabled = false;
    project.require_scan_sources().unwrap();
}

#[test]
fn reference_kept_visible_and_removing_it_from_order_does_not_remove_map() {
    let mut project = fixture();
    let id = project.order[0];
    assert!(!project.source(id).unwrap().reference_only);
    project.remove_from_filmstrip(id).unwrap();
    project.validate().unwrap();
    assert!(project.order.is_empty());
    assert!(project.flat_field().is_some());
    assert!(project.source(id).is_ok());
}
