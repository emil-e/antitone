#![cfg(feature = "project")]
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    color::Balance,
    processing::{Calibration, DEFAULT_GAMMA, InputSettings, Response},
    project::*,
};
use glam::DVec3;

fn fixture(directory: &std::path::Path) -> (Project, SourceId, SourceId) {
    let decode = Development::new("0.22.0".into(), [1.; 4]);
    let mut project = Project {
        input: InputSettings::Negative {
            dmin: Some(DVec3::new(
                0.16512039312008006,
                0.13295850981449217,
                0.11888115148085542,
            )),
            dmax: Some(DVec3::new(
                0.008358001578451848,
                0.003545242472549229,
                0.001551173905987176,
            )),
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        },
        ..Default::default()
    };
    let source = |name: &str| {
        let path = directory.join(name);
        std::fs::write(&path, b"test source bytes").unwrap();
        Source {
            capture_time: None,
            development_mode: antitone::project::DevelopmentMode::Unity,
            id: SourceId(0),
            path: name.into(),
            development: SourceDevelopment {
                decode: antitone::project::Development {
                    white_balance: [1.; 4],
                    ..decode.clone()
                },
                dimensions: [900, 600],
                active_area: [12, 12, 864, 576],
                camera_model: "test-camera".into(),
                camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            },
            adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
                AdjustmentLayerSettings::default(),
            )),
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        }
    };
    let dmin_source_id = project.add_source(source("dmin.ARW")).unwrap();
    let dmax_source_id = project.add_source(source("dmax.ARW")).unwrap();
    if let InputSettings::Negative { balance, .. } = &mut project.input {
        *balance = Balance {
            warmth: 0.12345678901234567,
            tint: -0.23456789012345678,
        };
    }
    project
        .source_mut(dmin_source_id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo = AdjustmentLayerSettings {
        exposure: 0.5,
        wb: Balance {
            warmth: 0.1,
            tint: -0.05,
        },
        ..AdjustmentLayerSettings::default()
    };
    (project, dmin_source_id, dmax_source_id)
}

#[test]
fn incomplete_calibration_still_validates_its_curve_controls() {
    let directory = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(directory.path());
    for (gamma, supported) in [(DEFAULT_GAMMA, true), (0., false)] {
        if let InputSettings::Negative {
            dmax, gamma: value, ..
        } = &mut project.input
        {
            *dmax = None;
            *value = gamma;
        }
        assert_eq!(serde_json::to_value(&project).is_ok(), supported);
    }
}

#[test]
fn libraw_release_provenance_does_not_gate_project_or_source_compatibility() {
    let directory = tempfile::tempdir().unwrap();
    let (project, _, _) = fixture(directory.path());
    let basis = project.sources[0].development.clone();
    for version in ["0.22.0", "0.22.3", "future decoder", ""] {
        let mut changed = project.clone();
        changed.sources[0].development.decode.backend_version = version.into();
        serde_json::to_value(&changed).unwrap();
        assert!(basis.same_processing(&changed.sources[0].development));
        if version != basis.decode.backend_version {
            assert_ne!(
                basis, changed.sources[0].development,
                "provenance remains part of full identity used by caches"
            );
        }
    }
    let mut encoded = serde_json::to_value(&project).unwrap();
    encoded["sources"][0]["development"]["decode"]
        .as_object_mut()
        .unwrap()
        .remove("backend_version");
    let unrecorded: Project = serde_json::from_value(encoded).unwrap();
    serde_json::to_value(&unrecorded).unwrap();
    assert!(
        unrecorded.sources[0]
            .development
            .decode
            .backend_version
            .is_empty()
    );
    let mut encoded = serde_json::to_value(&project).unwrap();
    encoded["sources"][0]["development"]["decode"]["backend"] = "unknown_backend".into();
    assert!(serde_json::from_value::<Project>(encoded).is_err());
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
    assert!(serde_json::to_value(&duplicate).is_err());
    let mut unknown = project.clone();
    unknown.analysis_regions[0].source = SourceId(999);
    assert!(serde_json::to_value(&unknown).is_err());
    let mut draft = project.clone();
    draft.analysis_regions[0].areas.picture = None;
    assert!(serde_json::to_value(&draft).is_err());
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
    assert!(serde_json::to_value(&invalid).is_err());
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
    serde_json::to_value(&legacy).unwrap();
}

#[test]
fn load_rejects_nonordered_source_records() {
    let directory = tempfile::tempdir().unwrap();
    let (mut project, kept, removed) = fixture(directory.path());
    let mut malformed = serde_json::to_value(&project).unwrap();
    malformed["order"] = serde_json::json!([kept]);
    project.order = vec![kept];
    assert!(serde_json::to_value(&project).is_err());
    let path = directory.path().join("nonordered.antitone");
    std::fs::write(&path, serde_json::to_vec_pretty(&malformed).unwrap()).unwrap();
    assert!(Project::load(&path).is_err());
    project.sources.retain(|source| source.id != removed);
    project.save(&path, Save::Replace).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
}

#[test]
fn input_stage_roundtrips() {
    let directory = tempfile::tempdir().unwrap();
    let (mut project, _id, _other) = fixture(directory.path());
    // The active stage persists and round-trips exactly.
    assert!(project.input.is_negative());
    let roundtrip: Project =
        serde_json::from_value(serde_json::to_value(&project).unwrap()).unwrap();
    assert_eq!(roundtrip.input, project.input);
    project.input = InputSettings::Digital;
    let roundtrip: Project =
        serde_json::from_value(serde_json::to_value(&project).unwrap()).unwrap();
    assert_eq!(roundtrip.input, InputSettings::Digital);
}

#[test]
fn photographic_settings_roundtrip_and_absent_fields_preserve_current_grades() {
    let directory = tempfile::tempdir().unwrap();
    let (mut project, id, _) = fixture(directory.path());
    let before = project
        .source(id)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0]
        .adjustment
        .photo;
    let photo = AdjustmentLayerSettings {
        contrast: 1.923456789012345,
        pivot: 0.23456789012345,
        contrast_colour_mix: 0.3456789012345,
        shadows: 2.56789012345,
        highlights: -2.6789012345,
        saturation: 1.23456789012345,
        vibrance: 0.456789012345,
        ..before
    };
    project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo = photo;
    let path = directory.path().join("grade.antitone.json");
    project.save(&path, Save::New).unwrap();
    assert_eq!(
        Project::load(&path)
            .unwrap()
            .source(id)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment
            .photo,
        photo
    );
    let mut json = serde_json::to_value(&project).unwrap();
    for source in json["sources"].as_array_mut().unwrap() {
        let fields = source["adjustment_layers"]["layers"][0]["adjustment"]["photo"]
            .as_object_mut()
            .unwrap();
        for name in [
            "contrast",
            "pivot",
            "contrast_colour_mix",
            "shadows",
            "highlights",
            "saturation",
            "vibrance",
        ] {
            fields.remove(name);
        }
    }
    let old: Project = serde_json::from_value(json).unwrap();
    serde_json::to_value(&old).unwrap();
    assert_eq!(
        old.source(id)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment
            .photo,
        before
    );
    project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .contrast = 2_f64.next_up();
    assert!(serde_json::to_value(&project).is_err());
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
    let calibration = project.calibration().unwrap().unwrap();
    let numerical = (
        project.roll,
        calibration.dmin.to_array(),
        calibration.dmax.to_array(),
        project
            .source(one)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment
            .photo,
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
            restored.calibration().unwrap().unwrap().dmin.to_array(),
            restored.calibration().unwrap().unwrap().dmax.to_array(),
            restored
                .source(one)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
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
                        loaded.calibration().unwrap().unwrap(),
                        loaded.processing_model
                    ),
                    (
                        project.roll,
                        project.calibration().unwrap().unwrap(),
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
        assert!(serde_json::to_value(&project).is_err());
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
        assert!(serde_json::to_value(&rotated_project).is_err());
    }
    let mut replacement = restored.clone();
    let before = replacement.source(one).unwrap().clone();
    let small = SourceDevelopment {
        dimensions: [64, 64],
        active_area: [0, 0, 64, 64],
        ..before.development.clone()
    };
    replacement.source_mut(one).unwrap().development = small;
    assert!(serde_json::to_value(&replacement).is_err());
    assert_eq!(restored.source(one).unwrap(), &before);
    let mut value = serde_json::to_value(&restored).unwrap();
    value["sources"][0]["crop"]["basis"] = serde_json::json!(1.);
    assert!(serde_json::from_value::<Project>(value).is_err());
}

#[test]
fn rec2020_peak_presets_roundtrip_as_project_rendering_intent() {
    use antitone::color::OutputTarget;

    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::default();
    assert_eq!(project.output.display_target, OutputTarget::DisplayP3);
    for target in OutputTarget::HDR {
        project.output.display_target = target;
        let path = directory
            .path()
            .join(format!("{}.antitone", target.peak_nits()));
        project.save(&path, Save::New).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
    }
}

#[test]
fn atomic_roundtrip_preserves_calibration_and_output_after_removal_and_missing_sources() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, dmin_source_id, dmax_source_id) = fixture(dir.path());
    let calibration = project.calibration().unwrap().unwrap();
    let before = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(
            project
                .source(dmax_source_id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo,
        ),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            project.input,
            project.roll,
            layers,
            glam::DMat3::from_cols_array_2d(
                &project
                    .source(dmax_source_id)
                    .unwrap()
                    .development
                    .camera_to_ap1,
            ),
        )
    })
    .unwrap()
    .apply(DVec3::new(0.08, 0.04, 0.015), [0, 0])
    .unwrap();
    project.remove_from_filmstrip(dmin_source_id).unwrap();
    assert!(project.source(dmin_source_id).is_err());
    project.order.reverse();
    let path = dir.path().join("roll.antitone.json");
    project.save(&path, Save::New).unwrap();
    std::fs::remove_file(dir.path().join("dmin.ARW")).unwrap();
    std::fs::remove_file(dir.path().join("dmax.ARW")).unwrap();
    let restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert_eq!(restored.calibration().unwrap().unwrap(), calibration);
    let after = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(
            restored
                .source(dmax_source_id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo,
        ),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(
            restored.input,
            restored.roll,
            layers,
            glam::DMat3::from_cols_array_2d(
                &restored
                    .source(dmax_source_id)
                    .unwrap()
                    .development
                    .camera_to_ap1,
            ),
        )
    })
    .unwrap()
    .apply(DVec3::new(0.08, 0.04, 0.015), [0, 0])
    .unwrap();
    assert_eq!(after, before);
    assert_eq!(restored.order, vec![dmax_source_id]);
    assert!(serde_json::to_value(&restored).is_ok());
}

#[test]
fn references_are_rgb_only_and_survive_removal_of_all_source_records() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(dir.path());
    let calibration = project.calibration().unwrap();
    let mut value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["schema"], antitone::project::SCHEMA_VERSION);
    assert_eq!(
        value["processing_model"],
        antitone::PROCESSING_MODEL_VERSION
    );
    assert!(value["sources"][0].get("fingerprint").is_none());
    assert_eq!(
        value["input"]["dmin"],
        serde_json::json!(project.calibration().unwrap().unwrap().dmin.to_array())
    );
    assert_eq!(
        value["input"]["dmax"],
        serde_json::json!(project.calibration().unwrap().unwrap().dmax.to_array())
    );
    assert_eq!(value["input"]["dmin"].as_array().unwrap().len(), 3);
    assert_eq!(value["input"]["dmax"].as_array().unwrap().len(), 3);
    value["input"]["dmin"] = serde_json::json!({"value": project.calibration().unwrap().unwrap().dmin.to_array(), "source": 1, "region": [0, 0, 32, 32]});
    assert!(serde_json::from_value::<Project>(value).is_err());
    project.sources.clear();
    project.order.clear();
    serde_json::to_value(&project).unwrap();
    assert_eq!(project.calibration().unwrap(), calibration);
    let path = dir.path().join("without-sources.antitone.json");
    project.save(&path, Save::New).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
}

#[test]
fn project_persists_display_target_without_codec_settings() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(dir.path());
    project.output.display_target = antitone::color::OutputTarget::Srgb;

    let value = serde_json::to_value(&project).unwrap();
    let output = value["output"].as_object().unwrap();
    assert_eq!(output["display_target"], "srgb");
    assert!(!output.contains_key("tiff"));
    assert!(!output.contains_key("exr"));
    let restored: Project = serde_json::from_value(value).unwrap();
    assert_eq!(restored, project);
}

#[test]
fn output_contract_and_old_schemas_are_rejected_without_reinterpretation() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _, _) = fixture(dir.path());
    assert_eq!(
        project.output.display_target,
        antitone::color::OutputTarget::DisplayP3
    );
    let mut value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["output"]["display_target"], "display_p3");
    value["output"]["display_target"] = "pro_photo".into();
    assert!(serde_json::from_value::<Project>(value).is_err());
    let path = dir.path().join("legacy.antitone.json");
    for schema in 1..antitone::project::SCHEMA_VERSION {
        let mut legacy = serde_json::to_value(&project).unwrap();
        legacy["schema"] = schema.into();
        assert!(serde_json::from_value::<Project>(legacy.clone()).is_err());
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(Project::load(&path).is_err());
    }
    let mut legacy = serde_json::to_value(&project).unwrap();
    legacy["output"]["contract"] = "ocio-2.6.0-aces2-100nit-antitone-srgb-prophoto-v1".into();
    assert!(serde_json::from_value::<Project>(legacy.clone()).is_err());
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(Project::load(&path).is_err());
}

#[test]
fn project_round_trips_custom_order_and_optional_capture_time() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(dir.path());
    let id = project.order[0];
    project.order.reverse();
    project.source_mut(id).unwrap().capture_time = Some(20250228144433u64.try_into().unwrap());
    let mut stored = serde_json::to_value(&project).unwrap();
    let reopened: Project = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(reopened, project);

    stored["sources"][0]["capture_time"] = 20250230144433u64.into();
    assert!(serde_json::from_value::<Project>(stored.clone()).is_err());

    stored["sources"][0]
        .as_object_mut()
        .unwrap()
        .remove("capture_time");
    let reopened: Project = serde_json::from_value(stored).unwrap();
    assert_eq!(reopened.order, project.order);
    assert!(
        reopened
            .sources
            .iter()
            .all(|source| source.capture_time.is_none())
    );
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
fn source_locators_and_decoder_provenance_preserve_calibration_while_unknown_models_reject() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, dmin_source_id, _) = fixture(dir.path());
    let moved = dir.path().join("renamed.ARW");
    std::fs::rename(dir.path().join("dmin.ARW"), &moved).unwrap();
    project.source_mut(dmin_source_id).unwrap().path = moved.clone();
    assert_eq!(project.source(dmin_source_id).unwrap().path, moved);
    let calibration = project.calibration().unwrap();
    project.sources[0].development.decode.backend_version = "different decoder release".into();
    serde_json::to_value(&project).unwrap();
    assert_eq!(project.calibration().unwrap(), calibration);
    let mut incompatible = serde_json::to_value(&project).unwrap();
    incompatible["processing_model"] = (antitone::PROCESSING_MODEL_VERSION + 1).into();
    assert!(serde_json::from_value::<Project>(incompatible).is_err());
}

#[test]
fn prior_processing_models_are_rejected_instead_of_reinterpreting_saved_grades() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _, _) = fixture(dir.path());
    let mut encoded = serde_json::to_value(&project).unwrap();
    encoded["processing_model"] = (antitone::PROCESSING_MODEL_VERSION - 1).into();
    let path = dir.path().join("prior-model.antitone");
    std::fs::write(&path, serde_json::to_vec(&encoded).unwrap()).unwrap();
    let error = Project::load(&path).unwrap_err().to_string();
    assert!(error.contains("unsupported processing model"), "{error}");
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
    if let InputSettings::Negative { gamma, .. } = &mut project.input {
        *gamma = f64::NAN;
    }
    assert!(project.save(&path, Save::Replace).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    if let InputSettings::Negative { gamma, .. } = &mut project.input {
        *gamma = 0.75;
    }
    project.save(&path, Save::Replace).unwrap();
    assert_eq!(Project::load(&path).unwrap(), project);
    let mut unsupported = serde_json::to_value(&project).unwrap();
    unsupported["schema"] = (antitone::project::SCHEMA_VERSION + 1).into();
    assert!(serde_json::from_value::<Project>(unsupported.clone()).is_err());
    std::fs::write(&path, serde_json::to_vec(&unsupported).unwrap()).unwrap();
    assert!(Project::load(&path).is_err());
}

#[test]
fn extended_white_balance_roundtrips_only_at_roll_scope() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, source, _) = fixture(dir.path());
    let extended = Balance {
        warmth: -1.25,
        tint: 1.5,
    };
    project.roll.wb = extended;
    let saved = serde_json::to_value(&project).unwrap();
    assert_eq!(
        serde_json::from_value::<Project>(saved.clone()).unwrap(),
        project
    );

    let mut invalid = saved;
    invalid["roll"]["wb"]["warmth"] = (-1.51).into();
    assert!(serde_json::from_value::<Project>(invalid).is_err());
    project
        .source_mut(source)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .wb = extended;
    assert!(serde_json::to_value(&project).is_err());
    project
        .source_mut(source)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .wb = Balance::default();
    let InputSettings::Negative { balance, .. } = &mut project.input else {
        panic!("negative fixture")
    };
    *balance = extended;
    assert!(serde_json::to_value(&project).is_err());
}

#[test]
fn malformed_source_identity_order_and_rgb_references_are_rejected_before_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let (project, source_id, _) = fixture(dir.path());
    let mut duplicate = project.clone();
    duplicate.order.push(source_id);
    assert!(serde_json::to_value(&duplicate).is_err());
    let mut identity = serde_json::to_value(&project).unwrap();
    identity["sources"][0]["fingerprint"] =
        serde_json::json!({"bytes": 1, "blake3": "a".repeat(64)});
    assert!(serde_json::from_value::<Project>(identity).is_err());
    let mut numeric = project.clone();
    numeric
        .source_mut(source_id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .wb
        .warmth = f64::INFINITY;
    assert!(serde_json::to_value(&numeric).is_err());
    let mut dimensions = project.clone();
    dimensions
        .source_mut(source_id)
        .unwrap()
        .development
        .active_area = [u32::MAX, 0, u32::MAX, 1];
    assert!(serde_json::to_value(&dimensions).is_err());
    for rgb in [[f64::NAN, 0.1, 0.1], [0., 0.1, 0.1], [-1e-8, 0.1, 0.1]] {
        let mut partial = project.clone();
        partial.input = InputSettings::Negative {
            dmin: Some(DVec3::from_array(rgb)),
            dmax: None,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        };
        assert!(serde_json::to_value(&partial).is_err());
    }
    let mut uncalibrated = project.clone();
    if let InputSettings::Negative { dmax, .. } = &mut uncalibrated.input {
        *dmax = None;
    }
    serde_json::to_value(&uncalibrated).unwrap();
}

#[test]
fn incompatible_reference_pairs_roundtrip_without_admitting_undefined_inversion() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _, _) = fixture(dir.path());
    let path = dir.path().join("recalibration.antitone");
    for (index, (dmin, dmax)) in [(0.1, 0.1), (0.1, 0.2), (0.1, 0.1 - 1e-9)]
        .into_iter()
        .enumerate()
    {
        let calibration = Calibration {
            dmin: DVec3::splat(dmin),
            dmax: DVec3::splat(dmax),
        };
        project.input = InputSettings::negative(calibration);
        assert!(antitone::processing::Inversion::new(calibration).is_err());
        project
            .save(&path, if index == 0 { Save::New } else { Save::Replace })
            .unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
    }
}

fn mixed_source(path: &str, camera: &str, red_gain: f64, matrix: [[f64; 3]; 3]) -> Source {
    let path = std::path::PathBuf::from(path);
    std::fs::write(&path, b"mixed source bytes").unwrap();
    Source {
        capture_time: None,
        development_mode: antitone::project::DevelopmentMode::Unity,
        id: SourceId(0),
        path,
        development: SourceDevelopment {
            decode: Development::new("0.22.0".into(), [red_gain, 1., 1., 1.]),
            dimensions: [900, 600],
            active_area: [0, 0, 900, 600],
            camera_model: camera.into(),
            camera_to_ap1: matrix,
        },
        adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
            AdjustmentLayerSettings::default(),
        )),
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    }
}

#[test]
fn digital_recipes_and_unresolved_negative_intent_preserve_mixed_camera_baselines() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project {
        input: InputSettings::Digital,
        ..Default::default()
    };
    let first = project
        .add_source(mixed_source(
            directory.path().join("camera-a.ARW").to_str().unwrap(),
            "camera-a",
            2.,
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ))
        .unwrap();
    let second = project
        .add_source(mixed_source(
            directory.path().join("camera-b.ARW").to_str().unwrap(),
            "camera-b",
            2.1,
            [[0.9, 0., 0.1], [0., 1.1, 0.], [0.05, 0., 0.95]],
        ))
        .unwrap();

    assert_eq!(
        project.source(first).unwrap().development.camera_model,
        "camera-a"
    );
    assert_eq!(
        project
            .source(second)
            .unwrap()
            .development
            .decode
            .white_balance,
        [2.1, 1., 1., 1.]
    );
    assert_eq!(
        project
            .source(second)
            .unwrap()
            .development
            .camera_basis()
            .camera_model,
        "camera-b"
    );
    let recipe = project.image_recipe(second).unwrap();
    assert_eq!(
        recipe.neutral.source_development.resolved().unwrap(),
        &project.source(second).unwrap().development
    );
    assert_eq!(recipe.neutral.input, InputSettings::Digital);
    assert_eq!(
        project
            .image_recipe(first)
            .unwrap()
            .neutral
            .source_development
            .resolved()
            .unwrap(),
        &project.source(first).unwrap().development
    );

    let baselines = project.sources.clone();
    project.input = InputSettings::default();
    serde_json::to_value(&project).unwrap();
    for id in [first, second] {
        let recipe = project.image_recipe(id).unwrap();
        assert!(recipe.neutral.input.is_negative());
        assert!(recipe.neutral.source_development.resolved().is_err());
    }
    assert_eq!(project.sources, baselines);
    let path = directory.path().join("unresolved.antitone");
    project.save(&path, Save::New).unwrap();
    let reopened = Project::load(&path).unwrap();
    assert_eq!(reopened, project);
    assert_eq!(reopened.sources, baselines);
}

#[test]
fn camera_descriptor_depends_on_coordinates_not_decoder_provenance() {
    let directory = tempfile::tempdir().unwrap();
    let source = mixed_source(
        directory.path().join("basis.ARW").to_str().unwrap(),
        "camera-a",
        1.,
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    );
    let basis = source.development.camera_basis();
    let mut other_version = source.development.clone();
    other_version
        .decode
        .backend_version
        .push_str("-new-release");
    assert_eq!(basis, other_version.camera_basis());
    assert!(source.development.same_processing(&other_version));
    let mut other_basis = source.development;
    other_basis.camera_model.push_str(" other camera");
    assert_ne!(basis, other_basis.camera_basis());
}

#[test]
fn default_negative_preserves_each_mixed_camera_development() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::default();
    assert!(matches!(project.input, InputSettings::Negative { .. }));
    assert!(project.flat_field.is_none());

    let first = project
        .add_source(mixed_source(
            directory.path().join("roll-a.ARW").to_str().unwrap(),
            "camera-a",
            1.,
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ))
        .unwrap();
    assert_eq!(
        project.source(first).unwrap().development.camera_model,
        "camera-a"
    );

    project
        .add_source(mixed_source(
            directory.path().join("roll-b.ARW").to_str().unwrap(),
            "camera-a",
            1.,
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ))
        .unwrap();

    let input = project.input;
    let third = project
        .add_source(mixed_source(
            directory.path().join("camera-b.ARW").to_str().unwrap(),
            "camera-b",
            1.,
            [[0.9, 0., 0.1], [0., 1.1, 0.], [0.05, 0., 0.95]],
        ))
        .unwrap();
    assert_eq!(project.input, input);
    assert_eq!(
        project
            .image_recipe(third)
            .unwrap()
            .neutral
            .source_development
            .resolved()
            .unwrap(),
        &project.source(third).unwrap().development
    );
}
