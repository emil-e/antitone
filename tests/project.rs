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
            local_adjustments: None,
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
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
        ..PhotoSettings::default()
    };
    (project, dmin_source_id, dmax_source_id)
}

#[test]
fn accepted_per_source_analysis_support_roundtrips_and_rejects_invalid_association_or_drafts() {
    use antitone::{coverage::Coverage, scan_regions::Areas};
    let directory = tempfile::tempdir().unwrap();
    let (mut project, id, other) = fixture(directory.path());
    let prior = project.source(other).unwrap().clone();
    let active = project.source(id).unwrap().development.active_area.into();
    let mut areas = Areas::default();
    areas.edit_opening(Coverage::rectangle(active));
    areas.edit_picture(Coverage::rectangle(active));
    project
        .analysis_regions
        .push(SourceAnalysis { source: id, areas });
    let path = directory.path().join("areas.antitone.json");
    project.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
    assert_eq!(project.source(other).unwrap(), &prior);
    let mut old = serde_json::to_value(&project).unwrap();
    old.as_object_mut().unwrap().remove("analysis_regions");
    let old: Project = serde_json::from_value(old).unwrap();
    assert!(old.analysis_regions.is_empty());
    assert_eq!(old.source(other).unwrap(), &prior);
    let mut duplicate = project.clone();
    duplicate
        .analysis_regions
        .push(duplicate.analysis_regions[0].clone());
    assert!(duplicate.validate().is_err());
    let mut unknown = project.clone();
    unknown.analysis_regions[0].source = SourceId(999);
    assert!(unknown.validate().is_err());
    let mut draft = project.clone();
    draft.analysis_regions[0].areas.picture = None;
    assert!(draft.validate().is_err());
    let mut malformed = serde_json::to_value(&project).unwrap();
    malformed["analysis_regions"][0]["areas"]["unknown_support"] = true.into();
    assert!(serde_json::from_value::<Project>(malformed).is_err());
    let a = &mut project.analysis_regions[0].areas;
    a.opening_edited = false;
    a.opening_shared = true;
    a.picture_edited = false;
    a.picture_geometry = Some(antitone::scan_regions::PictureGeometry {
        size: [768, 512],
        angle: 0.,
        sides: [None; 4],
    });
    let path = directory.path().join("shared-areas.antitone.json");
    project.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
    let mut invalid = project.clone();
    invalid.analysis_regions[0]
        .areas
        .picture_geometry
        .as_mut()
        .unwrap()
        .sides[0] = Some(1.);
    assert!(invalid.validate().is_err());
    let mut legacy = serde_json::to_value(&project).unwrap();
    let a = legacy["analysis_regions"][0]["areas"]
        .as_object_mut()
        .unwrap();
    for key in ["opening_shared", "opening_exception", "picture_geometry"] {
        a.remove(key);
    }
    let legacy: Project = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.analysis_regions[0].areas.opening_shared);
    assert!(legacy.analysis_regions[0].areas.picture_geometry.is_none());
    legacy.validate().unwrap();
}

#[test]
fn photographic_settings_roundtrip_and_absent_fields_preserve_current_grades() {
    let directory = tempfile::tempdir().unwrap();
    let (mut project, id, _) = fixture(directory.path());
    let before = project.source(id).unwrap().photo;
    let photo = PhotoSettings {
        contrast: 1.123456789012345,
        pivot: 0.23456789012345,
        contrast_colour_mix: 0.3456789012345,
        saturation: 1.23456789012345,
        saturation_density: 0.456789012345,
        ..before
    };
    project.source_mut(id).unwrap().photo = photo;
    let path = directory.path().join("grade.antitone.json");
    project.save(&path, Save::New).unwrap();
    assert_eq!(
        Project::load(&path).unwrap().source(id).unwrap().photo,
        photo
    );
    let mut json = serde_json::to_value(&project).unwrap();
    for source in json["sources"].as_array_mut().unwrap() {
        let fields = source["photo"].as_object_mut().unwrap();
        for name in [
            "contrast",
            "pivot",
            "contrast_colour_mix",
            "saturation",
            "saturation_density",
        ] {
            fields.remove(name);
        }
    }
    let old: Project = serde_json::from_value(json).unwrap();
    old.validate().unwrap();
    assert_eq!(old.source(id).unwrap().photo, before);
    project.source_mut(id).unwrap().photo.contrast = 2.;
    assert!(project.validate().is_err());
}

#[test]
fn orientation_roundtrip_keeps_legacy_identity_and_numerical_state() {
    use antitone::geometry::{Flips, PhotoOrientation, QuarterTurns};
    let dir = tempfile::tempdir().unwrap();
    let (mut project, one, two) = fixture(dir.path());
    let path = dir.path().join("geometry.antitone.json");
    project.save(&path, Save::New).unwrap();
    let legacy = std::fs::read_to_string(&path).unwrap();
    assert!(!legacy.contains("orientation"));
    assert!(!legacy.contains("crop"));
    assert_eq!(Project::load(&path).unwrap(), project);
    let numerical = (
        project.roll,
        project.dmin,
        project.dmax,
        project.source(one).unwrap().photo,
    );
    project.orientation = Flips {
        horizontal: true,
        vertical: false,
    }
    .into();
    let photo = PhotoOrientation {
        turns: QuarterTurns::Three,
        flips: Flips {
            horizontal: false,
            vertical: true,
        },
    };
    project.source_mut(one).unwrap().orientation = photo;
    project.order.reverse();
    project.save(&path, Save::Replace).unwrap();
    let restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert_eq!(restored.source(one).unwrap().orientation, photo);
    assert_eq!(
        restored.source(two).unwrap().orientation,
        PhotoOrientation::default()
    );
    assert_eq!(
        (
            restored.roll,
            restored.dmin,
            restored.dmax,
            restored.source(one).unwrap().photo
        ),
        numerical
    );
    let invalid = std::fs::read_to_string(&path)
        .unwrap()
        .replace("three", "four");
    std::fs::write(&path, invalid).unwrap();
    assert!(Project::load(&path).is_err());
}

#[test]
fn roll_turns_preserve_flip_only_documents_and_roundtrip_without_changing_photos_or_grade() {
    use antitone::geometry::{Flips, QuarterTurns, RollOrientation};
    let dir = tempfile::tempdir().unwrap();
    let (project, _, _) = fixture(dir.path());
    let original = serde_json::to_value(&project).unwrap();
    let path = dir.path().join("roll-turns.json");
    for horizontal in [false, true] {
        for vertical in [false, true] {
            let legacy = serde_json::json!({"horizontal":horizontal,"vertical":vertical});
            let mut value = original.clone();
            value["orientation"] = legacy.clone();
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            let mut loaded = Project::load(&path).unwrap();
            assert_eq!(
                loaded.orientation,
                RollOrientation {
                    turns: QuarterTurns::Zero,
                    horizontal,
                    vertical
                }
            );
            if horizontal || vertical {
                assert_eq!(
                    serde_json::to_value(&loaded).unwrap()["orientation"],
                    legacy
                );
            } else {
                assert!(
                    serde_json::to_value(&loaded)
                        .unwrap()
                        .get("orientation")
                        .is_none()
                );
            }
            for turns in [
                QuarterTurns::Zero,
                QuarterTurns::One,
                QuarterTurns::Two,
                QuarterTurns::Three,
            ] {
                loaded.orientation.turns = turns;
                loaded.save(&path, Save::Replace).unwrap();
                assert_eq!(Project::load(&path).unwrap(), loaded);
                let serialized = serde_json::to_value(&loaded).unwrap();
                if turns != QuarterTurns::Zero {
                    assert!(
                        serde_json::from_value::<Flips>(serialized["orientation"].clone()).is_err()
                    );
                }
                assert_eq!(loaded.sources, project.sources);
                assert_eq!(
                    (
                        loaded.roll,
                        loaded.dmin,
                        loaded.dmax,
                        loaded.processing_model
                    ),
                    (
                        project.roll,
                        project.dmin,
                        project.dmax,
                        project.processing_model
                    )
                );
            }
        }
    }
    for bad in [
        serde_json::json!({"turns":"four","horizontal":false,"vertical":false}),
        serde_json::json!({"turns":90,"horizontal":false,"vertical":false}),
        serde_json::json!({"turns":"one","horizontal":false,"vertical":false,"extra":0}),
    ] {
        let mut value = original.clone();
        value["orientation"] = bad;
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Project::load(&path).is_err());
    }
}

#[test]
fn crops_roundtrip_by_source_id_and_reject_outside_fractional_or_empty_coverage() {
    use antitone::{crop::Crop, geometry::SourceRegion};
    let dir = tempfile::tempdir().unwrap();
    let (mut project, one, two) = fixture(dir.path());
    let crop = Crop::from_region(SourceRegion {
        x: 123,
        y: 77,
        width: 321,
        height: 213,
    });
    let source = project.source(one).unwrap().clone();
    project.source_mut(one).unwrap().crop = Some(crop);
    project.order.reverse();
    let path = dir.path().join("crop.json");
    project.save(&path, Save::New).unwrap();
    let restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert_eq!(restored.source(one).unwrap().crop, Some(crop));
    assert!(restored.source(two).unwrap().crop.is_none());
    let mut without_crop = restored.source(one).unwrap().clone();
    without_crop.crop = None;
    assert_eq!(without_crop, source);
    for invalid in [
        Crop {
            center: [0., 0.],
            ..crop
        },
        Crop {
            size: [0, 10],
            ..crop
        },
        Crop {
            center: [f64::INFINITY, 77.],
            ..crop
        },
        Crop {
            center: [123.25, 77.],
            size: [10, 10],
            angle: 0.,
        },
    ] {
        project.source_mut(one).unwrap().crop = Some(invalid);
        assert!(project.validate().is_err());
    }
    // Zero angle keeps the literal old centre/extent representation.
    assert!(
        serde_json::to_value(&restored).unwrap()["sources"][0]["crop"]
            .get("angle")
            .is_none()
    );
    let active: SourceRegion = restored.source(one).unwrap().development.active_area.into();
    let rotated = Crop::from_region(active)
        .straightened(active, 0.231)
        .unwrap();
    let mut rotated_project = restored.clone();
    rotated_project.source_mut(one).unwrap().crop = Some(rotated);
    rotated_project
        .save(&dir.path().join("rotated.json"), Save::New)
        .unwrap();
    let loaded_rotated = Project::load(&dir.path().join("rotated.json")).unwrap();
    assert_eq!(loaded_rotated.source(one).unwrap().crop, Some(rotated));
    for invalid in [
        Crop {
            angle: f64::NAN,
            ..rotated
        },
        Crop {
            angle: 4.,
            ..rotated
        },
        Crop {
            center: [0., 0.],
            ..rotated
        },
    ] {
        rotated_project.source_mut(one).unwrap().crop = Some(invalid);
        assert!(rotated_project.validate().is_err());
    }
    let mut replacement = restored.clone();
    let before = replacement.source(one).unwrap().clone();
    let small = SourceDevelopment {
        dimensions: [64, 64],
        active_area: [0, 0, 64, 64],
        ..before.development.clone()
    };
    replacement.source_mut(one).unwrap().development = small;
    assert!(replacement.validate().is_err());
    assert_eq!(restored.source(one).unwrap(), &before);
    let mut value = serde_json::to_value(&restored).unwrap();
    value["sources"][0]["crop"]["basis"] = serde_json::json!(1.);
    assert!(serde_json::from_value::<Project>(value).is_err());
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
    assert_eq!(value["schema"], 6);
    assert_eq!(value["processing_model"], 4);
    assert!(value["sources"][0].get("fingerprint").is_none());
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
    for schema in [1, 2, 3, 4] {
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
fn changed_photo_bytes_do_not_change_saved_state_or_invalidate_calibration() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _, _) = fixture(dir.path());
    let calibration = project.calibration().unwrap();
    let path = dir.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    std::fs::write(dir.path().join("dmin.ARW"), b"changed file data").unwrap();
    let restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert_eq!(restored.calibration().unwrap(), calibration);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("fingerprint")
    );
}

#[test]
fn source_locators_keep_stable_ids_and_runtime_identity_is_checked_at_roll_level() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, dmin_source_id, _) = fixture(dir.path());
    let moved = dir.path().join("renamed.ARW");
    std::fs::rename(dir.path().join("dmin.ARW"), &moved).unwrap();
    project.source_mut(dmin_source_id).unwrap().path = moved.clone();
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
    let mut identity = serde_json::to_value(&project).unwrap();
    identity["sources"][0]["fingerprint"] =
        serde_json::json!({"bytes": 1, "blake3": "a".repeat(64)});
    assert!(serde_json::from_value::<Project>(identity).is_err());
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
