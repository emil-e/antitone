#![cfg(feature = "project")]

use antitone::{
    dust_repair::{DonorChoice, RepairId, RepairOrigin, RepairSet, SupportMask, evaluate},
    project::{Development, Project, Save, Source, SourceDevelopment, SourceId},
};
use serde_json::json;

fn fixture() -> (Project, SourceId) {
    let mut project = Project {
        development: Some(Development::new("0.22.0".into(), [1.; 4])),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "negative.ARW".into(),
            development: SourceDevelopment {
                dimensions: [900, 600],
                active_area: [12, 12, 864, 576],
                camera_model: "test-camera".into(),
                camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
            },
            reference_only: false,
            photo: Default::default(),
            repairs: RepairSet::default(),
            local_adjustments: None,
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    (project, id)
}

fn with_repairs() -> (Project, SourceId) {
    let (mut project, id) = fixture();
    let set = &mut project.source_mut(id).unwrap().repairs;
    // Keep the declared rectangle, including blank margins, holes and disconnected pixels.
    let bits = vec![
        false, true, false, false, true, true, false, false, false, false, false, true,
    ];
    set.insert(
        SupportMask::new([60, 60], [4, 3], bits).unwrap(),
        RepairOrigin::Manual,
        DonorChoice::Automatic,
    )
    .unwrap();
    let manual = set
        .insert(
            SupportMask::new([150, 60], [2, 2], vec![true; 4]).unwrap(),
            RepairOrigin::Detected,
            DonorChoice::Offset([-40, 30]),
        )
        .unwrap();
    set.set_enabled(manual, false).unwrap();
    (project, id)
}

#[test]
fn exact_instructions_survive_single_file_roundtrip_and_deleted_ids_are_not_reused() {
    let (mut project, id) = with_repairs();
    project.remove_from_filmstrip(id).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dust.antitone");
    project.save(&path, Save::New).unwrap();
    let mut loaded = Project::load(&path).unwrap();
    assert_eq!(loaded, project);

    let wire = serde_json::to_value(&loaded).unwrap();
    let support = &wire["sources"][0]["repairs"]["repairs"][0]["support"];
    assert_eq!(support["runs"], json!([[1, 1], [4, 2], [11, 1]]));
    assert_eq!(support["origin"], json!([60, 60]));
    assert_eq!(support["size"], json!([4, 3]));
    assert!(support.get("bits").is_none());

    let set = &mut loaded.source_mut(id).unwrap().repairs;
    set.clear();
    loaded.save(&path, Save::Replace).unwrap();
    let mut reopened = Project::load(&path).unwrap();
    let next = reopened
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([60, 60], [1, 1], vec![true]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    assert_eq!(next, RepairId(3));
    reopened.order.push(id);
    reopened.validate().unwrap();
}

#[test]
fn malformed_and_unsupported_repairs_reject_without_replacing_an_existing_file() {
    let (project, _) = with_repairs();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.antitone");
    project.save(&path, Save::New).unwrap();
    let original = std::fs::read(&path).unwrap();
    let baseline = serde_json::to_value(&project).unwrap();
    let changes = [
        ("/schema", json!(5)),
        ("/sources/0/repairs/contract", json!("future-healer")),
        (
            "/sources/0/repairs/contract",
            json!("graded-ap1-boundary-ranked-score10-grow4-v1"),
        ),
        ("/sources/0/repairs/last_id", json!(0)),
        ("/sources/0/repairs/repairs/0/id", json!(0)),
        ("/sources/0/repairs/repairs/1/id", json!(1)),
        ("/sources/0/repairs/repairs/0/support/runs", json!([])),
        ("/sources/0/repairs/repairs/0/support/runs", json!([[1, 0]])),
        (
            "/sources/0/repairs/repairs/0/support/runs",
            json!([[1, 2], [2, 2]]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/runs",
            json!([[1, 2], [3, 1]]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/runs",
            json!([[4, 1], [1, 1]]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/runs",
            json!([[11, 2]]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/runs",
            json!([[1, u32::MAX]]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/size",
            json!([u32::MAX, 2]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/size",
            json!([513, 512]),
        ),
        (
            "/sources/0/repairs/repairs/0/support/origin",
            json!([u32::MAX, 1]),
        ),
        ("/sources/0/repairs/repairs/0/support/origin", json!([0, 0])),
        (
            "/sources/0/repairs/repairs/1/support/origin",
            json!([60, 60]),
        ),
        (
            "/sources/0/repairs/repairs/1/donor",
            json!({"offset": [0, 0]}),
        ),
        (
            "/sources/0/repairs/repairs/1/donor",
            json!({"offset": [-1000, 0]}),
        ),
        (
            "/sources/0/repairs/repairs/0/origin",
            json!("future_detector"),
        ),
        (
            "/sources/0/repairs/repairs/0/support",
            json!({"origin": [20, 20], "size": [300, 300], "runs": [[0, 65537]]}),
        ),
        ("/sources/0/repairs/repairs/0/support/size", json!([0, 3])),
        (
            "/sources/0/repairs/repairs/0/donor",
            json!({"offset": [1, 1], "unknown": 1}),
        ),
    ];

    for (pointer, value) in changes {
        let mut wire = baseline.clone();
        *wire.pointer_mut(pointer).unwrap() = value;
        if let Ok(candidate) = serde_json::from_value::<Project>(wire) {
            assert!(candidate.validate().is_err(), "{pointer}");
            assert!(candidate.save(&path, Save::Replace).is_err(), "{pointer}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
fn all_repair_fields_are_required_and_unknown_fields_reject() {
    let (project, _) = with_repairs();
    let baseline = serde_json::to_value(&project).unwrap();
    for pointer in [
        "/sources/0",
        "/sources/0/repairs",
        "/sources/0/repairs/repairs/0",
        "/sources/0/repairs/repairs/0/support",
    ] {
        let object = baseline.pointer(pointer).unwrap().as_object().unwrap();
        let required: Vec<_> = if pointer == "/sources/0" {
            vec!["repairs".to_owned()]
        } else {
            object.keys().cloned().collect()
        };

        for key in required {
            let mut wire = baseline.clone();
            wire.pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(&key);
            assert!(
                serde_json::from_value::<Project>(wire).is_err(),
                "{pointer}/{key}"
            );
        }

        let mut wire = baseline.clone();
        wire.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), json!(true));
        assert!(
            serde_json::from_value::<Project>(wire).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn schema_five_reports_unsupported_schema_instead_of_missing_repair_state() {
    let (project, _) = fixture();
    let mut wire = serde_json::to_value(project).unwrap();
    wire["schema"] = json!(5);
    wire["sources"][0]
        .as_object_mut()
        .unwrap()
        .remove("repairs");
    let error = serde_json::from_value::<Project>(wire).unwrap_err();
    assert!(
        error.to_string().contains("unsupported project schema"),
        "{error}"
    );
}

#[test]
fn exhausted_allocator_is_retained_but_cannot_wrap_or_reuse_an_id() {
    let (project, id) = with_repairs();
    let mut wire = serde_json::to_value(&project).unwrap();
    wire["sources"][0]["repairs"]["last_id"] = json!(u64::MAX);
    let mut loaded: Project = serde_json::from_value(wire).unwrap();
    loaded.validate().unwrap();
    let before = loaded.clone();
    assert!(
        loaded
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(
                SupportMask::new([300, 60], [1, 1], vec![true]).unwrap(),
                RepairOrigin::Manual,
                DonorChoice::Automatic
            )
            .is_err()
    );
    assert_eq!(loaded, before);
}

#[test]
fn automatic_recomputation_reproduces_donors_and_f32_bits_without_serialized_results() {
    let (project, id) = with_repairs();
    let source = project.source(id).unwrap();
    let base = |[x, y]: [u32; 2]| Ok([x as f32 * 0.001 - 0.2, y as f32 * 0.002, 2.]);
    let before = evaluate(
        &source.repairs.repairs()[0],
        source.development.active_area.into(),
        base,
    )
    .unwrap();

    let wire = serde_json::to_vec(&project).unwrap();
    let loaded: Project = serde_json::from_slice(&wire).unwrap();
    loaded.validate().unwrap();
    let source = loaded.source(id).unwrap();
    let after = evaluate(
        &source.repairs.repairs()[0],
        source.development.active_area.into(),
        base,
    )
    .unwrap();
    assert_eq!(before.donors, after.donors);
    assert_eq!(
        before
            .pixels()
            .map(|(p, v)| (p, v.map(f32::to_bits)))
            .collect::<Vec<_>>(),
        after
            .pixels()
            .map(|(p, v)| (p, v.map(f32::to_bits)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn uneven_36_photo_roll_with_fragmented_masks_saves_beyond_the_former_eight_mib_limit() {
    let (mut project, id) = fixture();
    let template = project.source(id).unwrap().clone();
    for index in 1..36 {
        let mut source = template.clone();
        source.path = format!("photo-{index}.ARW").into();
        project.add_source(source).unwrap();
    }
    for source in project.sources.iter_mut().take(3) {
        for index in 0..6 {
            let bits = (0..128 * 128)
                .map(|i| (i % 128 + i / 128) % 2 == 0)
                .collect();
            source
                .repairs
                .insert(
                    SupportMask::new(
                        [20 + (index % 3) * 140, 20 + (index / 3) * 140],
                        [128, 128],
                        bits,
                    )
                    .unwrap(),
                    RepairOrigin::Manual,
                    DonorChoice::Automatic,
                )
                .unwrap();
        }
    }

    let dusty = &mut project.sources[3];
    for index in 0..3000 {
        dusty
            .repairs
            .insert(
                SupportMask::new(
                    [20 + (index % 80) * 10, 20 + (index / 80) * 10],
                    [1, 1],
                    vec![true],
                )
                .unwrap(),
                RepairOrigin::Manual,
                DonorChoice::Automatic,
            )
            .unwrap();
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roll.antitone");
    project.save(&path, Save::New).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 8 * 1024 * 1024);
    assert_eq!(Project::load(&path).unwrap(), project);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
