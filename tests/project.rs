#![cfg(feature = "project")]
use antitone::{
    color::Balance,
    processing::{PhotoSettings, Pipeline},
    project::*,
};
use glam::DVec3;

fn fixture(directory: &std::path::Path) -> (Project, SourceId, SourceId) {
    let mut project = Project {
        development: Some(Development::new(
            "0.22.0".into(),
            [2133., 1024., 1771., 1024.],
        )),
        input_floor: Some(antitone::processing::CAMERA_INPUT_FLOOR),
        dmin: Some([
            0.16512039312008006,
            0.13295850981449217,
            0.11888115148085542,
        ]),
        dmax: Some([
            0.008358001578451848,
            0.003545242472549229,
            0.001551173905987176,
        ]),
        ..Default::default()
    };
    let source = |name: &str| {
        let path = directory.join(name);
        std::fs::write(&path, b"test source bytes").unwrap();
        Source {
            id: SourceId(0),
            fingerprint: Fingerprint::read(&path).unwrap(),
            path: name.into(),
            development: SourceDevelopment {
                dimensions: [900, 600],
                active_area: [12, 12, 864, 576],
                camera_model: "test-camera".into(),
                camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
            },
            reference_only: false,
            photo: PhotoSettings::default(),
        }
    };
    let dmin_source_id = project.add_source(source("dmin.ARW")).unwrap();
    let dmax_source_id = project.add_source(source("dmax.ARW")).unwrap();
    project.roll.balance = Balance {
        warmth: 0.12345678901234567,
        tint: -0.23456789012345678,
    };
    project.source_mut(dmin_source_id).unwrap().photo = PhotoSettings {
        exposure: 0.5,
        wb: Balance {
            warmth: 0.1,
            tint: -0.05,
        },
    };
    (project, dmin_source_id, dmax_source_id)
}
#[test]
fn atomic_roundtrip_preserves_calibration_and_output_after_removal_and_missing_sources() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, dmin_source_id, dmax_source_id) = fixture(dir.path());
    let calibration = project.calibration().unwrap().unwrap();
    let before = Pipeline::new(
        calibration,
        project.roll,
        project.source(dmin_source_id).unwrap().photo,
        glam::DMat3::from_cols_array_2d(
            &project
                .source(dmin_source_id)
                .unwrap()
                .development
                .camera_to_ap1,
        ),
    )
    .unwrap()
    .apply(DVec3::new(0.08, 0.04, 0.015))
    .unwrap();
    project.remove_from_filmstrip(dmin_source_id).unwrap();
    project.order.reverse();
    project.source_mut(dmax_source_id).unwrap().reference_only = true;
    let path = dir.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    std::fs::remove_file(dir.path().join("dmin.ARW")).unwrap();
    std::fs::remove_file(dir.path().join("dmax.ARW")).unwrap();
    let restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert_eq!(
        restored.inspect_source(dmin_source_id, &path).unwrap(),
        SourceStatus::Missing
    );
    assert_eq!(restored.calibration().unwrap().unwrap(), calibration);
    let after = Pipeline::new(
        restored.calibration().unwrap().unwrap(),
        restored.roll,
        restored.source(dmin_source_id).unwrap().photo,
        glam::DMat3::from_cols_array_2d(
            &restored
                .source(dmin_source_id)
                .unwrap()
                .development
                .camera_to_ap1,
        ),
    )
    .unwrap()
    .apply(DVec3::new(0.08, 0.04, 0.015))
    .unwrap();
    assert_eq!(after, before);
    assert_eq!(restored.order, vec![dmax_source_id]);
    assert!(restored.require_runtime("0.22.0").is_ok());
    assert!(restored.require_runtime("new decoder").is_err());
}
#[test]
fn references_are_rgb_only_and_survive_removal_of_all_source_records() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(dir.path());
    let calibration = project.calibration().unwrap();
    let mut value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["schema"], 4);
    assert_eq!(value["dmin"], serde_json::json!(project.dmin));
    assert_eq!(value["dmax"], serde_json::json!(project.dmax));
    assert_eq!(value["dmin"].as_array().unwrap().len(), 3);
    assert_eq!(value["dmax"].as_array().unwrap().len(), 3);
    value["dmin"] =
        serde_json::json!({"value": project.dmin, "source": 1, "region": [0, 0, 32, 32]});
    assert!(serde_json::from_value::<Project>(value).is_err());
    project.sources.clear();
    project.order.clear();
    project.validate().unwrap();
    assert_eq!(project.calibration().unwrap(), calibration);
    let path = dir.path().join("without-sources.antitone.json");
    project.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
}
#[test]
fn output_contract_and_old_schemas_are_rejected_without_reinterpretation() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _, _) = fixture(dir.path());
    assert_eq!(project.output.tiff, TiffTarget::DisplayP3);
    let mut value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["output"]["tiff"], "display_p3");
    value["output"]["tiff"] = "pro_photo".into();
    assert!(serde_json::from_value::<Project>(value).is_err());
    let path = dir.path().join("legacy.antitone.json");
    for schema in [1, 2] {
        let mut legacy = project.clone();
        legacy.schema = schema;
        assert!(legacy.validate().is_err());
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(Project::load(&path).is_err());
    }
    let mut legacy = project;
    legacy.output.contract = "ocio-2.6.0-aces2-100nit-antitone-srgb-prophoto-v1".into();
    legacy.validate().unwrap();
    assert!(legacy.require_runtime("0.22.0").is_err());
}
#[test]
fn changed_photo_bytes_require_acceptance_without_invalidating_calibration() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, dmin_source_id, _) = fixture(dir.path());
    let calibration = project.calibration().unwrap();
    let path = dir.path().join("roll.antitone.json");
    std::fs::write(dir.path().join("dmin.ARW"), b"changed file data").unwrap();
    assert!(matches!(
        project.inspect_source(dmin_source_id, &path).unwrap(),
        SourceStatus::Changed { .. }
    ));
    let original = project.clone();
    assert!(
        project
            .relink_matching(dmin_source_id, &dir.path().join("dmin.ARW"))
            .is_err()
    );
    assert_eq!(project, original);
    let mut development = project.source(dmin_source_id).unwrap().development.clone();
    development.dimensions = [64, 64];
    development.active_area = [0, 0, 64, 64];
    project
        .accept_changed_source(dmin_source_id, &dir.path().join("dmin.ARW"), development)
        .unwrap();
    assert_eq!(project.dmin, original.dmin);
    assert_eq!(project.dmax, original.dmax);
    assert_eq!(project.calibration().unwrap(), calibration);
    project.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
}
#[test]
fn matching_relink_keeps_stable_ids_and_runtime_identity_is_checked_at_roll_level() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, dmin_source_id, _) = fixture(dir.path());
    let moved = dir.path().join("renamed.ARW");
    std::fs::rename(dir.path().join("dmin.ARW"), &moved).unwrap();
    project.relink_matching(dmin_source_id, &moved).unwrap();
    assert_eq!(project.source(dmin_source_id).unwrap().path, moved);
    let calibration = project.calibration().unwrap();
    project.development.as_mut().unwrap().contract = "different development contract".into();
    assert!(project.require_runtime("0.22.0").is_err());
    assert_eq!(project.calibration().unwrap(), calibration);
    project.validate().unwrap();
    project.processing_model += 1;
    assert!(project.require_runtime("0.22.0").is_err());
}
#[test]
fn failed_save_and_unknown_schema_never_damage_the_previous_document() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(dir.path());
    let path = dir.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    let original = std::fs::read(&path).unwrap();
    project.roll.exposure = 1.;
    assert!(project.save(&path, Save::New).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    project.roll.gamma = f64::NAN;
    assert!(project.save(&path, Save::Replace).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    project.roll.gamma = 0.75;
    project.save(&path, Save::Replace).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
    project.schema += 1;
    assert!(project.validate().is_err());
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    assert!(Project::load(&path).is_err());
}
#[test]
fn malformed_source_identity_order_and_rgb_references_are_rejected_before_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let (project, source_id, _) = fixture(dir.path());
    let mut duplicate = project.clone();
    duplicate.order.push(source_id);
    assert!(duplicate.validate().is_err());
    let mut identity = project.clone();
    identity.source_mut(source_id).unwrap().fingerprint.blake3 = "invalid".into();
    assert!(identity.validate().is_err());
    let mut numeric = project.clone();
    numeric.source_mut(source_id).unwrap().photo.wb.warmth = f64::INFINITY;
    assert!(numeric.validate().is_err());
    let mut dimensions = project.clone();
    dimensions
        .source_mut(source_id)
        .unwrap()
        .development
        .active_area = [u32::MAX, 0, u32::MAX, 1];
    assert!(dimensions.validate().is_err());
    for rgb in [[f64::NAN, 0.1, 0.1], [0., 0.1, 0.1], [1e-8, 0.1, 0.1]] {
        let mut partial = project.clone();
        partial.dmin = Some(rgb);
        partial.dmax = None;
        assert!(partial.validate().is_err());
    }
    let mut ordering = project.clone();
    ordering.dmax = ordering.dmin;
    assert!(ordering.validate().is_err());
    let mut missing_floor = project.clone();
    missing_floor.input_floor = None;
    missing_floor.dmax = None;
    assert!(missing_floor.validate().is_err());
    let mut missing_development = project;
    missing_development.development = None;
    missing_development.sources.clear();
    missing_development.order.clear();
    assert!(missing_development.validate().is_err());
}
