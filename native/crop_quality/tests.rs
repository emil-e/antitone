use super::*;
use crate::{
    geometry::SourcePoint,
    project::{Development, Save, Source, SourceAnalysis, SourceDevelopment},
    scan_regions::{Areas, PictureGeometry},
};

fn fixture(directory: &Path) -> PathBuf {
    let mut project = Project {
        development: Some(Development::new("authored".into(), [1.; 4])),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "unavailable.ARW".into(),
            reference_only: false,
            development: SourceDevelopment {
                dimensions: [256, 160],
                active_area: [12, 20, 200, 120],
                camera_model: "authored".into(),
                camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            },
            photo: Default::default(),
            repairs: Default::default(),
            local_adjustments: None,
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    let active = project.source(id).unwrap().development.active_area.into();
    let geometry = PictureGeometry {
        size: [200, 120],
        angle: 0.,
        sides: [Some(22.), Some(182.), Some(30.), Some(100.)],
    };
    project.analysis_regions.push(SourceAnalysis {
        source: id,
        areas: Areas {
            opening: Some(Coverage::rectangle(active).corners().map(|p| p.0)),
            picture: geometry.frame(active),
            picture_geometry: Some(geometry),
            picture_observed: [true; 4],
            picture_constraints: vec![
                [1., 0., 34.],
                [-1., 0., -194.],
                [0., 1., 50.],
                [0., -1., -120.],
            ],
            ..Default::default()
        },
    });
    project
        .save(&directory.join("roll.antitone"), Save::New)
        .unwrap();
    let manifest = directory.join("marks.json");
    fs::write(&manifest,serde_json::to_vec(&json!({
        "margins_percent":[0,0.5],
        "cases":[{"project":"roll.antitone","source":id,"observation":"visible_frame",
            "marks":{"wanted":[[[34,50],[36,50],[36,52],[34,52]]],"unwanted":[[[32,50],[34,50],[34,52],[32,52]]]},
            "windows":[[12,20,32,24]]}]
    })).unwrap()).unwrap();
    manifest
}

#[test]
fn project_report_preserves_pose_marked_content_tradeoffs_and_read_only_state_without_raw_or_grade()
{
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let project_path = directory.path().join("roll.antitone");
    let before = fs::read(&project_path).unwrap();
    let mut inputs = Inputs::default();
    let (cases, margins) = load(&path, &mut inputs).unwrap();
    assert!(cases[0].project.calibration().unwrap().is_none());
    assert_eq!(
        cases[0]
            .project
            .resolve_path(cases[0].spec.source, &cases[0].project_path)
            .unwrap(),
        directory.path().join("unavailable.ARW")
    );
    let result = report(&cases, &margins).unwrap();
    let proposals = result["cases"][0]["proposals"].as_array().unwrap();
    assert_eq!(proposals[0]["status"], "complete_proposal");
    assert_eq!(proposals[0]["crop"]["center"], json!([114., 85.]));
    assert_eq!(proposals[0]["wanted_lost"]["error_source_pixel_area"], 0.);
    assert_eq!(
        proposals[0]["unwanted_admitted"]["error_source_pixel_area"],
        0.
    );
    // A 0.35px requested margin rounds inward by one full pixel on each axis.
    // Only one of this 2x2 corner mark's four square pixels survives.
    assert_eq!(proposals[1]["wanted_lost"]["error_source_pixel_area"], 3.);
    assert_eq!(proposals[1]["wanted_lost"]["marked_source_pixel_area"], 4.);
    assert!(proposals[1]["dense_wanted_lost"]["error_source_pixel_area"].is_null());
    inputs.verify().unwrap();
    assert_eq!(fs::read(project_path).unwrap(), before);
}

#[test]
fn censored_labels_flag_proposals_but_do_not_override_the_normal_evidence_gate() {
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (mut cases, margins) = load(&path, &mut Inputs::default()).unwrap();
    cases[0].spec.observation = Observation::Censored;
    let result = report(&cases, &margins).unwrap();
    assert_eq!(
        result["cases"][0]["proposals"][0]["proposal_on_uncertain_source"],
        true
    );
    let areas = &mut cases[0].project.analysis_regions[0].areas;
    areas.picture_observed[2] = false;
    areas.picture_geometry.as_mut().unwrap().sides[2] = None;
    areas.picture_constraints.remove(2);
    cases[0].project.validate().unwrap();
    let result = report(&cases, &margins).unwrap();
    let proposal = &result["cases"][0]["proposals"][0];
    assert_eq!(proposal["status"], "abstain");
    assert!(proposal.get("wanted_lost").is_none());
    assert!(proposal.get("crop").is_none());
}

#[test]
fn retained_evidence_does_not_make_a_removed_or_reference_source_a_photo_proposal() {
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let (mut cases, margins) = load(&path, &mut Inputs::default()).unwrap();
    let id = cases[0].spec.source;
    let ordinary = cases[0].project.clone();
    assert!(proposal(&cases[0]).is_ok());
    cases[0].project.remove_from_filmstrip(id).unwrap();
    cases[0].project.validate().unwrap();
    assert_eq!(
        report(&cases, &margins).unwrap()["cases"][0]["proposals"][0]["status"],
        "abstain"
    );
    cases[0].project = ordinary;
    cases[0].project.source_mut(id).unwrap().reference_only = true;
    cases[0].project.validate().unwrap();
    assert!(cases[0].project.order.contains(&id));
    assert!(proposal(&cases[0]).is_err());
}

#[test]
fn overlap_counts_actual_rotated_coverage_not_its_bounding_box() {
    let mark =
        Coverage::new([[10., 20.], [20., 20.], [20., 30.], [10., 30.]].map(SourcePoint)).unwrap();
    let crop = Crop {
        center: [15., 25.],
        size: [10, 10],
        angle: 0.,
    };
    assert!((overlap(crop, mark) - 100.).abs() < 1e-9);
    assert!(
        (overlap(
            Crop {
                center: [20., 25.],
                ..crop
            },
            mark
        ) - 50.)
            .abs()
            < 1e-9
    );
    assert_eq!(
        overlap(
            Crop {
                center: [100., 100.],
                ..crop
            },
            mark
        ),
        0.
    );
    let rotated = Crop {
        angle: 0.37,
        ..crop
    };
    let independent: f64 = (20..30)
        .flat_map(|y| (10..20).map(move |x| [x, y]))
        .map(|p| rotated.coverage().unwrap().weight(p))
        .sum();
    assert!((overlap(rotated, mark) - independent).abs() < 1e-9);
    assert!(overlap(rotated, mark) < 100.);
    let empty = score(rotated, &[], true).unwrap();
    assert_eq!(empty.marked_source_pixel_area, 0.);
    assert!(empty.error_source_pixel_area.is_none());

    // Small marked areas near a large frame's far corner must not vanish through cancellation.
    let far = 3_999_999_990.;
    let far_mark = Coverage::new(
        [
            [far, far],
            [far + 1., far],
            [far + 1., far + 1.],
            [far, far + 1.],
        ]
        .map(SourcePoint),
    )
    .unwrap();
    let large = Crop {
        center: [2_000_000_000.; 2],
        size: [4_000_000_000; 2],
        angle: 0.,
    };
    assert_eq!(overlap(large, far_mark), 1.);
}

#[test]
fn invalid_marks_windows_margins_and_case_identity_fail_before_any_source_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let valid: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut invalids = vec![];
    let mut duplicate = valid.clone();
    duplicate["cases"]
        .as_array_mut()
        .unwrap()
        .push(valid["cases"][0].clone());
    invalids.push(duplicate);
    let mut unknown = valid.clone();
    unknown["cases"][0]["guessed_policy"] = json!(true);
    invalids.push(unknown);
    let mut source = valid.clone();
    source["cases"][0]["source"] = json!(999);
    invalids.push(source);
    let mut margin = valid.clone();
    margin["margins_percent"] = json!([0, -1]);
    invalids.push(margin);
    let mut duplicate_margin = valid.clone();
    duplicate_margin["margins_percent"] = json!([0, 0]);
    invalids.push(duplicate_margin);
    let mut mark = valid.clone();
    mark["cases"][0]["marks"]["wanted"] = json!([[[0, 0], [1, 0], [1, 1], [0, 1]]]);
    invalids.push(mark);
    let mut window = valid.clone();
    window["cases"][0]["windows"][0][0] = json!(212);
    invalids.push(window);
    let mut oversize = valid.clone();
    oversize["cases"][0]["windows"][0][2] = json!(257);
    invalids.push(oversize);
    let mut empty = valid.clone();
    empty["cases"] = json!([]);
    invalids.push(empty);
    for invalid in invalids {
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(load(&path, &mut Inputs::default()).is_err());
    }
}

#[cfg(unix)]
#[test]
fn aliased_project_keeps_normal_relative_source_resolution_and_checks_its_locator() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let aliases = directory.path().join("aliases");
    fs::create_dir(&aliases).unwrap();
    symlink("../roll.antitone", aliases.join("roll.antitone")).unwrap();
    let manifest = aliases.join("marks.json");
    fs::copy(&path, &manifest).unwrap();
    let mut inputs = Inputs::default();
    let (cases, _) = load(&manifest, &mut inputs).unwrap();
    assert_eq!(
        cases[0]
            .project
            .resolve_path(cases[0].spec.source, &cases[0].project_path)
            .unwrap(),
        aliases.join("unavailable.ARW")
    );
    fs::copy(
        directory.path().join("roll.antitone"),
        directory.path().join("other.antitone"),
    )
    .unwrap();
    fs::remove_file(aliases.join("roll.antitone")).unwrap();
    symlink("../other.antitone", aliases.join("roll.antitone")).unwrap();
    assert!(inputs.verify().is_err());
}

#[cfg(unix)]
#[test]
fn manifest_locator_retargeting_does_not_pass_with_an_unchanged_old_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let alias = directory.path().join("manifest-alias.json");
    symlink("marks.json", &alias).unwrap();
    let mut inputs = Inputs::default();
    load(&alias, &mut inputs).unwrap();
    fs::copy(&path, directory.path().join("other-marks.json")).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink("other-marks.json", &alias).unwrap();
    assert!(inputs.verify().is_err());
}

#[test]
fn manifest_and_project_revisions_are_verified_before_publishing_results() {
    let directory = tempfile::tempdir().unwrap();
    let path = fixture(directory.path());
    let mut inputs = Inputs::default();
    load(&path, &mut inputs).unwrap();
    fs::write(directory.path().join("roll.antitone"), b"changed").unwrap();
    assert!(inputs.verify().is_err());
}
