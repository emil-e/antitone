#![cfg(feature = "project")]
use antitone::{
    geometry::SourcePoint,
    local_adjustments::{CONTRACT, GradientMask, LocalAdjustment, SpatialPipeline},
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    project::{
        AdjustmentId, Development, LocalAdjustments, Project, Save, Source, SourceDevelopment,
        SourceId,
    },
};
use glam::{DMat3, DVec3};

fn adjustment() -> LocalAdjustment {
    LocalAdjustment {
        mask: GradientMask::Linear {
            start: SourcePoint([-10.25, 20.125]),
            end: SourcePoint([900.5, 20.125]),
        },
        photo: PhotoSettings {
            exposure: 1.123456789012345,
            contrast: 1.2,
            ..PhotoSettings::default()
        },
        enabled: true,
        invert: false,
        strength: 0.765432109876543,
    }
}

fn fixture() -> (Project, SourceId) {
    let mut project = Project {
        development: Some(Development::new("test-backend".into(), [2., 1., 1.5, 1.])),
        ..Project::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "one.ARW".into(),
            development: SourceDevelopment {
                lens_correction: None,
                dng_development: None,
                dimensions: [900, 600],
                active_area: [12, 7, 864, 576],
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
        })
        .unwrap();
    (project, id)
}

fn with_locals() -> (Project, SourceId) {
    let (mut project, id) = fixture();
    let mut locals = LocalAdjustments::default();
    locals.add("Linear 1".into(), adjustment()).unwrap();
    locals
        .add(
            "Radial 1".into(),
            LocalAdjustment {
                mask: GradientMask::Radial {
                    center: SourcePoint([401.1234567890123, 299.9876543210987]),
                    radii: [201.2345678901234, 87.6543210987654],
                    angle: 0.2345678901234567,
                    feather: 0.6543210987654321,
                },
                invert: true,
                photo: PhotoSettings {
                    saturation: 0.6,
                    saturation_density: 0.4,
                    ..PhotoSettings::default()
                },
                ..adjustment()
            },
        )
        .unwrap();
    project.source_mut(id).unwrap().local_adjustments = Some(locals);
    (project, id)
}

#[test]
fn absent_local_state_remains_optional_in_schema_six() {
    let (project, id) = fixture();
    let json = serde_json::to_value(&project).unwrap();
    assert_eq!(json["schema"], 6);
    assert!(json["sources"][0].get("local_adjustments").is_none());
    let loaded: Project = serde_json::from_value(json).unwrap();
    loaded.validate().unwrap();
    assert_eq!(loaded, project);
    assert!(
        loaded
            .source(id)
            .unwrap()
            .grade_snapshot()
            .unwrap()
            .locals()
            .is_empty()
    );
}

#[test]
fn source_owned_local_records_roundtrip_exactly_without_recomputing_masks() {
    let (mut project, id) = with_locals();
    let mut other = project.source(id).unwrap().clone();
    other.path = "two.ARW".into();
    other.local_adjustments = None;
    let other_id = project.add_source(other).unwrap();
    project.order.reverse();
    project
        .source_mut(id)
        .unwrap()
        .local_adjustments
        .as_mut()
        .unwrap()
        .adjustments
        .reverse();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grade.json");
    project.save(&path, Save::New).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded, project);
    assert_eq!(
        loaded.source(id).unwrap().grade_snapshot().unwrap(),
        project.source(id).unwrap().grade_snapshot().unwrap()
    );
    assert!(loaded.source(other_id).unwrap().local_adjustments.is_none());
    let json = serde_json::to_value(&loaded).unwrap();
    assert_eq!(
        json["sources"][0]["local_adjustments"]["contract"],
        CONTRACT
    );
    assert_eq!(
        json["sources"][0]["local_adjustments"]["adjustments"][0]["adjustment"]["mask"]["type"],
        "radial"
    );
}

#[test]
fn adjustment_ids_survive_rename_reorder_deletion_and_empty_collection_save() {
    let mut locals = LocalAdjustments::default();
    let one = locals.add("First".into(), adjustment()).unwrap();
    let two = locals.add("Second".into(), adjustment()).unwrap();
    locals.adjustments.reverse();
    locals.adjustments[0].name = "Renamed".into();
    assert_eq!(locals.adjustments[0].id, two);
    locals.remove(one).unwrap();
    locals.remove(two).unwrap();
    let (mut project, id) = fixture();
    project.source_mut(id).unwrap().local_adjustments = Some(locals);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty-locals.json");
    project.save(&path, Save::New).unwrap();
    let mut loaded = Project::load(&path).unwrap();
    let locals = loaded
        .source_mut(id)
        .unwrap()
        .local_adjustments
        .as_mut()
        .unwrap();
    assert!(locals.adjustments.is_empty());
    assert_eq!(locals.next_id, 3);
    assert_eq!(
        locals.add("Third".into(), adjustment()).unwrap(),
        AdjustmentId(3)
    );
}

#[test]
fn invalid_add_remove_and_exhausted_allocator_do_not_mutate_accepted_records() {
    let mut locals = LocalAdjustments::default();
    locals.add("First".into(), adjustment()).unwrap();
    let before = locals.clone();
    assert!(locals.add(" ".into(), adjustment()).is_err());
    assert!(
        locals
            .add(
                "Invalid".into(),
                LocalAdjustment {
                    strength: 2.,
                    ..adjustment()
                }
            )
            .is_err()
    );
    assert!(locals.remove(AdjustmentId(123)).is_err());
    assert_eq!(locals, before);
    locals.next_id = u64::MAX;
    let before = locals.clone();
    assert!(locals.add("Exhausted".into(), adjustment()).is_err());
    assert_eq!(locals, before);
}

#[test]
fn malformed_local_state_rejects_and_failed_save_preserves_existing_project() {
    let (project, _) = with_locals();
    let original = serde_json::to_value(&project).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grade.json");
    project.save(&path, Save::New).unwrap();
    let saved_bytes = std::fs::read(&path).unwrap();
    for (pointer, bad) in [
        (
            "/sources/0/local_adjustments/contract",
            serde_json::json!("unknown-contract"),
        ),
        ("/sources/0/local_adjustments/next_id", serde_json::json!(0)),
        (
            "/sources/0/local_adjustments/adjustments/0/id",
            serde_json::json!(0),
        ),
        (
            "/sources/0/local_adjustments/adjustments/1/id",
            serde_json::json!(1),
        ),
        (
            "/sources/0/local_adjustments/adjustments/0/name",
            serde_json::json!(" "),
        ),
        (
            "/sources/0/local_adjustments/adjustments/0/adjustment/strength",
            serde_json::json!(-0.1),
        ),
        (
            "/sources/0/local_adjustments/adjustments/0/adjustment/photo/contrast",
            serde_json::json!(2.),
        ),
        (
            "/sources/0/local_adjustments/adjustments/0/adjustment/mask/end",
            serde_json::json!([-10.25, 20.125]),
        ),
        (
            "/sources/0/local_adjustments/adjustments/1/adjustment/mask/radii",
            serde_json::json!([0., 1.]),
        ),
        (
            "/sources/0/local_adjustments/adjustments/1/adjustment/mask/feather",
            serde_json::json!(2.),
        ),
    ] {
        let mut json = original.clone();
        *json.pointer_mut(pointer).unwrap() = bad;
        let invalid: Project = serde_json::from_value(json).unwrap();
        assert!(invalid.validate().is_err(), "accepted {pointer}");
        assert!(invalid.save(&path, Save::Replace).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    }
    for pointer in [
        "/sources/0/local_adjustments",
        "/sources/0/local_adjustments/adjustments/0",
        "/sources/0/local_adjustments/adjustments/0/adjustment",
        "/sources/0/local_adjustments/adjustments/0/adjustment/mask",
        "/sources/0/local_adjustments/adjustments/0/adjustment/photo",
    ] {
        let mut json = original.clone();
        json.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), serde_json::json!(1));
        assert!(
            serde_json::from_value::<Project>(json).is_err(),
            "accepted unknown field at {pointer}"
        );
    }
    for key in ["contract", "next_id", "adjustments"] {
        let mut json = original.clone();
        json["sources"][0]["local_adjustments"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(serde_json::from_value::<Project>(json).is_err());
    }
}

#[test]
fn older_strict_source_readers_reject_local_state_instead_of_discarding_it() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OldSource {
        id: SourceId,
        path: std::path::PathBuf,
        development: SourceDevelopment,
        reference_only: bool,
        repairs: antitone::dust_repair::RepairSet,
        photo: PhotoSettings,
        #[serde(default)]
        orientation: antitone::geometry::PhotoOrientation,
        #[serde(default)]
        crop: Option<antitone::crop::Crop>,
    }
    let (project, id) = fixture();
    let source = project.source(id).unwrap();
    let old: OldSource = serde_json::from_value(serde_json::to_value(source).unwrap()).unwrap();
    assert_eq!(old.repairs, source.repairs);
    assert_eq!(
        (
            old.id,
            old.path,
            old.development,
            old.reference_only,
            old.photo,
            old.orientation,
            old.crop
        ),
        (
            source.id,
            source.path.clone(),
            source.development.clone(),
            source.reference_only,
            source.photo,
            source.orientation,
            source.crop
        )
    );
    let (project, id) = with_locals();
    assert!(
        serde_json::from_value::<OldSource>(
            serde_json::to_value(project.source(id).unwrap()).unwrap()
        )
        .is_err()
    );
}

#[test]
fn processing_snapshot_owns_values_and_ignores_names_ids_and_allocator_metadata() {
    let (mut project, id) = with_locals();
    let snapshot = project.source(id).unwrap().grade_snapshot().unwrap();
    let source = project.source_mut(id).unwrap();
    let locals = source.local_adjustments.as_mut().unwrap();
    locals.adjustments[0].name = "Renamed".into();
    locals.adjustments[0].id = AdjustmentId(3);
    locals.next_id = 4;
    assert_eq!(snapshot, source.grade_snapshot().unwrap());
    source.photo.exposure = 2.;
    source
        .local_adjustments
        .as_mut()
        .unwrap()
        .adjustments
        .reverse();
    assert_ne!(snapshot, source.grade_snapshot().unwrap());
    assert_eq!(snapshot.global(), PhotoSettings::default());
    assert_eq!(snapshot.locals()[0], adjustment());
}

#[test]
fn spatial_pipeline_uses_snapshot_and_full_source_pixel_centres() {
    let (mut project, id) = fixture();
    let settings = LocalAdjustment {
        mask: GradientMask::Linear {
            start: SourcePoint([12., 7.]),
            end: SourcePoint([16., 7.]),
        },
        photo: PhotoSettings {
            exposure: 2.,
            ..PhotoSettings::default()
        },
        strength: 1.,
        ..adjustment()
    };
    let mut locals = LocalAdjustments::default();
    locals.add("Exposure".into(), settings).unwrap();
    project.source_mut(id).unwrap().local_adjustments = Some(locals);
    let snapshot = project.source(id).unwrap().grade_snapshot().unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.02),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::new_grade();
    let global = Pipeline::new(calibration, roll, snapshot.global(), DMat3::IDENTITY).unwrap();
    let spatial = SpatialPipeline::new(calibration, roll, snapshot, DMat3::IDENTITY).unwrap();
    project.source_mut(id).unwrap().local_adjustments = None;
    let scan = DVec3::new(0., 0.1, 0.3);
    let before = global.apply(scan).unwrap();
    for x in 12..16 {
        let t = (f64::from(x) + 0.5 - 12.) / 4.;
        let w = t * t * (3. - 2. * t);
        let after = spatial.apply(scan, [x, 7]).unwrap();
        let expected = before.rgb * (2. * w).exp2();
        for c in 0..3 {
            assert!((after.rgb[c] - expected[c]).abs() <= 2e-12 + 2e-11 * expected[c].abs());
        }
        assert_eq!(after.floored, before.floored);
    }
    let empty = project.source(id).unwrap().grade_snapshot().unwrap();
    let empty = SpatialPipeline::new(calibration, roll, empty, DMat3::IDENTITY).unwrap();
    assert_eq!(empty.apply(scan, [12, 7]).unwrap(), before);
}
