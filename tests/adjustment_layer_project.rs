#![cfg(feature = "project")]
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerPipeline, CONTRACT,
    },
    geometry::SourcePoint,
    processing::{
        Calibration, DEFAULT_GAMMA, InputPipeline, InputSettings, Response, RollSettings,
    },
    project::{
        AdjustmentLayerId, AdjustmentLayers, Development, Project, Save, Source, SourceDevelopment,
        SourceId,
    },
};
use glam::{DMat3, DVec3};

fn adjustment() -> AdjustmentLayer {
    AdjustmentLayer {
        mask: AdjustmentLayerCoverage::Linear {
            start: SourcePoint([-10.25, 20.125]),
            end: SourcePoint([900.5, 20.125]),
        },
        photo: AdjustmentLayerSettings {
            exposure: 1.123456789012345,
            contrast: 1.2,
            ..AdjustmentLayerSettings::default()
        },
        enabled: true,
        invert: false,
        strength: 0.765432109876543,
    }
}

fn fixture() -> (Project, SourceId) {
    let mut project = Project::default();
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "one.ARW".into(),
            development: SourceDevelopment {
                decode: Development::new("test-backend".into(), [1.; 4]),
                lens_correction: None,
                dng_development: None,
                dimensions: [900, 600],
                active_area: [12, 7, 864, 576],
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
        })
        .unwrap();
    (project, id)
}

fn with_locals() -> (Project, SourceId) {
    let (mut project, id) = fixture();
    let mut layers = AdjustmentLayers::default();
    layers.add("Linear 1".into(), adjustment()).unwrap();
    layers
        .add(
            "Radial 1".into(),
            AdjustmentLayer {
                mask: AdjustmentLayerCoverage::Radial {
                    center: SourcePoint([401.1234567890123, 299.9876543210987]),
                    radii: [201.2345678901234, 87.6543210987654],
                    angle: 0.2345678901234567,
                    feather: 0.6543210987654321,
                },
                invert: true,
                photo: AdjustmentLayerSettings {
                    saturation: 0.6,
                    saturation_density: 0.4,
                    ..AdjustmentLayerSettings::default()
                },
                ..adjustment()
            },
        )
        .unwrap();
    project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    (project, id)
}

#[test]
fn initial_global_layer_is_serialized_as_an_ordinary_layer_and_empty_stacks_roundtrip() {
    let (mut project, id) = fixture();
    let json = serde_json::to_value(&project).unwrap();
    assert_eq!(json["schema"], antitone::project::SCHEMA_VERSION);
    assert!(json["sources"][0].get("photo").is_none());
    let mut malformed = json.clone();
    malformed["sources"][0]["adjustment_layers"]["layers"][0]["adjustment"]["mask"]["center"] =
        serde_json::json!([0., 0.]);
    assert!(
        serde_json::from_value::<Project>(malformed).is_err(),
        "global coverage must not silently discard unsupported geometry"
    );
    let snapshot = project.source(id).unwrap().grade_snapshot().unwrap();
    assert_eq!(
        snapshot.layers(),
        &[AdjustmentLayer::global(AdjustmentLayerSettings::default())]
    );
    assert_eq!(serde_json::from_value::<Project>(json).unwrap(), project);
    project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers
        .clear();
    let loaded: Project = serde_json::from_value(serde_json::to_value(&project).unwrap()).unwrap();
    loaded.validate().unwrap();
    assert!(
        loaded
            .source(id)
            .unwrap()
            .grade_snapshot()
            .unwrap()
            .layers()
            .is_empty()
    );
}

#[test]
fn source_owned_local_records_roundtrip_exactly_without_recomputing_masks() {
    let (mut project, id) = with_locals();
    let mut other = project.source(id).unwrap().clone();
    other.path = "two.ARW".into();
    other.adjustment_layers = None;
    let other_id = project.add_source(other).unwrap();
    project.order.reverse();
    project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers
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
    assert!(loaded.source(other_id).unwrap().adjustment_layers.is_none());
    let json = serde_json::to_value(&loaded).unwrap();
    assert_eq!(
        json["sources"][0]["adjustment_layers"]["contract"],
        CONTRACT
    );
    assert_eq!(
        json["sources"][0]["adjustment_layers"]["layers"][0]["adjustment"]["mask"]["type"],
        "radial"
    );
}

#[test]
fn adjustment_ids_survive_rename_reorder_deletion_and_empty_collection_save() {
    let mut layers = AdjustmentLayers::default();
    let one = layers.add("First".into(), adjustment()).unwrap();
    let two = layers.add("Second".into(), adjustment()).unwrap();
    layers.layers.reverse();
    layers.layers[0].name = "Renamed".into();
    assert_eq!(layers.layers[0].id, two);
    layers.remove(one).unwrap();
    layers.remove(two).unwrap();
    let (mut project, id) = fixture();
    project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty-layers.json");
    project.save(&path, Save::New).unwrap();
    let mut loaded = Project::load(&path).unwrap();
    let layers = loaded
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap();
    assert!(layers.layers.is_empty());
    assert_eq!(layers.next_id, 3);
    assert_eq!(
        layers.add("Third".into(), adjustment()).unwrap(),
        AdjustmentLayerId(3)
    );
}

#[test]
fn invalid_add_remove_and_exhausted_allocator_do_not_mutate_accepted_records() {
    let mut layers = AdjustmentLayers::default();
    layers.add("First".into(), adjustment()).unwrap();
    let before = layers.clone();
    assert!(layers.add(" ".into(), adjustment()).is_err());
    assert!(
        layers
            .add(
                "Invalid".into(),
                AdjustmentLayer {
                    strength: 2.,
                    ..adjustment()
                }
            )
            .is_err()
    );
    assert!(layers.remove(AdjustmentLayerId(123)).is_err());
    assert_eq!(layers, before);
    layers.next_id = u64::MAX;
    let before = layers.clone();
    assert!(layers.add("Exhausted".into(), adjustment()).is_err());
    assert_eq!(layers, before);
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
            "/sources/0/adjustment_layers/contract",
            serde_json::json!("unknown-contract"),
        ),
        ("/sources/0/adjustment_layers/next_id", serde_json::json!(0)),
        (
            "/sources/0/adjustment_layers/layers/0/id",
            serde_json::json!(0),
        ),
        (
            "/sources/0/adjustment_layers/layers/1/id",
            serde_json::json!(1),
        ),
        (
            "/sources/0/adjustment_layers/layers/0/name",
            serde_json::json!(" "),
        ),
        (
            "/sources/0/adjustment_layers/layers/0/adjustment/strength",
            serde_json::json!(-0.1),
        ),
        (
            "/sources/0/adjustment_layers/layers/0/adjustment/photo/contrast",
            serde_json::json!(2.),
        ),
        (
            "/sources/0/adjustment_layers/layers/0/adjustment/mask/end",
            serde_json::json!([-10.25, 20.125]),
        ),
        (
            "/sources/0/adjustment_layers/layers/1/adjustment/mask/radii",
            serde_json::json!([0., 1.]),
        ),
        (
            "/sources/0/adjustment_layers/layers/1/adjustment/mask/feather",
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
        "/sources/0/adjustment_layers",
        "/sources/0/adjustment_layers/layers/0",
        "/sources/0/adjustment_layers/layers/0/adjustment",
        "/sources/0/adjustment_layers/layers/0/adjustment/mask",
        "/sources/0/adjustment_layers/layers/0/adjustment/photo",
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
    for key in ["contract", "next_id", "layers"] {
        let mut json = original.clone();
        json["sources"][0]["adjustment_layers"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(serde_json::from_value::<Project>(json).is_err());
    }
}

#[test]

fn processing_snapshot_owns_values_and_ignores_names_ids_and_allocator_metadata() {
    let (mut project, id) = with_locals();
    let snapshot = project.source(id).unwrap().grade_snapshot().unwrap();
    let source = project.source_mut(id).unwrap();
    let layers = source.adjustment_layers.as_mut().unwrap();
    layers.layers[0].name = "Renamed".into();
    layers.layers[0].id = AdjustmentLayerId(3);
    layers.next_id = 4;
    assert_eq!(snapshot, source.grade_snapshot().unwrap());
    source.adjustment_layers.as_mut().unwrap().layers[0]
        .adjustment
        .photo
        .exposure = 2.;
    source.adjustment_layers.as_mut().unwrap().layers.reverse();
    assert_ne!(snapshot, source.grade_snapshot().unwrap());

    assert_eq!(snapshot.layers()[0], adjustment());
}

#[test]
fn spatial_pipeline_uses_snapshot_and_full_source_pixel_centres() {
    let (mut project, id) = fixture();
    let settings = AdjustmentLayer {
        mask: AdjustmentLayerCoverage::Linear {
            start: SourcePoint([12., 7.]),
            end: SourcePoint([16., 7.]),
        },
        photo: AdjustmentLayerSettings {
            exposure: 2.,
            ..AdjustmentLayerSettings::default()
        },
        strength: 1.,
        ..adjustment()
    };
    let mut layers = AdjustmentLayers::default();
    layers.add("Exposure".into(), settings).unwrap();
    project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    let snapshot = project.source(id).unwrap().grade_snapshot().unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.02),
        input_floor: 1. / 65535.,
    };
    let input = InputSettings::Negative {
        dmin: Some(calibration.dmin),
        dmax: Some(calibration.dmax),
        input_floor: calibration.input_floor,
        response: Response::film(),
        gamma: DEFAULT_GAMMA,
        balance: Default::default(),
    };
    let roll = RollSettings::default();
    let global = InputPipeline::new(input, roll, DMat3::IDENTITY).unwrap();
    let spatial = AdjustmentLayerPipeline::new(input, roll, snapshot, DMat3::IDENTITY).unwrap();
    project.source_mut(id).unwrap().adjustment_layers = None;
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
    let empty = AdjustmentLayerPipeline::new(input, roll, empty, DMat3::IDENTITY).unwrap();
    assert_eq!(empty.apply(scan, [12, 7]).unwrap(), before);
}
