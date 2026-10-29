use super::*;
use crate::{
    application::test_support::with_application,
    editor::{Pick, history::Direction},
    processing::Calibration,
    raw::Rect,
};
use std::time::Duration;

fn setup(path: &str) -> Editor {
    let mut editor = crate::editor::tests::authored_fixture(&[path]);
    editor.load_input(
        Calibration {
            dmin: DVec3::splat(0.2),
            dmax: DVec3::splat(0.005),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        }
        .into(),
    );
    editor.document.source = false;
    editor.sync_document();
    editor
}

fn patch(editor: &Editor) -> crate::coverage::Coverage {
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    crate::coverage::Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 32,
    })
}

#[test]
fn each_reference_feature_awaits_the_same_camera_measurement_and_changes_only_its_endpoint() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    for kind in [Pick::Dmin, Pick::Dmax] {
        let editor = setup(file.path().to_str().unwrap());
        let region = patch(&editor);
        let expected = measurement::capture_scan(&editor)
            .unwrap()
            .reference_coverage(region)
            .unwrap();
        assert!(expected.min_element() > 0.005 && expected.max_element() < 0.2);
        let before = editor.document.state.project.clone();
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.begin_pick(kind, cx));
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.document.state.project, before);
                assert!(!editor.busy);
            });
            app.enter(|editor, cx| {
                editor.set_pick_region(Some(region));
                editor.apply_pick(cx);
            });
            app.run_until(Duration::from_secs(5), |editor| !editor.busy);
            app.read(|editor| {
                let calibration = editor.document.input.calibration().unwrap();
                assert_eq!(
                    calibration.dmin,
                    if kind == Pick::Dmin {
                        expected
                    } else {
                        DVec3::splat(0.2)
                    }
                );
                assert_eq!(
                    calibration.dmax,
                    if kind == Pick::Dmax {
                        expected
                    } else {
                        DVec3::splat(0.005)
                    }
                );
                assert_eq!(
                    editor.history_label(Direction::Undo),
                    Some("Calibration pick")
                );
                assert!(editor.active_pick().is_none());
                assert!(!editor.document.source);
            });
        });
    }
}

#[test]
fn gamma_feature_fits_its_captured_negative_stage_without_changing_roll_or_layer_wb() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw/illuminated-patches.dng");
    let mut editor = setup(path.to_str().unwrap());
    let mut image = measurement::capture_scan(&editor).unwrap();
    let patch_at = |x| Rect {
        x,
        y: 16,
        width: 32,
        height: 32,
    };
    editor.load_input(
        Calibration {
            dmin: image.reference(patch_at(16)).unwrap(),
            dmax: image.reference(patch_at(80)).unwrap(),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        }
        .into(),
    );
    editor.document.roll.wb = Balance {
        warmth: 0.2,
        tint: -0.1,
    };
    editor.document.visit.adjustment_layer_settings.wb = Balance {
        warmth: -0.1,
        tint: 0.2,
    };
    editor.sync_document();
    let region = crate::coverage::Coverage::rectangle(patch_at(144));
    let input = editor.input();
    let expected = measurement::capture_scan(&editor)
        .unwrap()
        .roll_neutral_coverage(region, &input)
        .unwrap();
    let before = (
        editor.document.roll,
        editor.document.visit.adjustment_layer_settings,
    );
    assert_ne!(expected, editor.balance());
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::Gamma, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.balance(), expected);
            assert_eq!(
                (
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                ),
                before
            );
            assert_eq!(
                editor.gamma(),
                match input {
                    InputSettings::Negative { gamma, .. } => gamma,
                    _ => unreachable!(),
                }
            );
            assert_eq!(
                editor.history_label(Direction::Undo),
                Some("Calibration pick")
            );
        });
    });
}

#[test]
fn a_measured_reference_can_be_rejected_by_its_caller_without_history_and_the_next_pick_can_apply()
{
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = setup(file.path().to_str().unwrap());
    editor.load_input(
        Calibration {
            dmin: DVec3::splat(2.),
            dmax: DVec3::ONE,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        }
        .into(),
    );
    editor.sync_document();
    let region = patch(&editor);
    let expected = measurement::capture_scan(&editor)
        .unwrap()
        .reference_coverage(region)
        .unwrap();
    assert!(expected.min_element() > 0.005 && expected.max_element() < 1.);
    let before = editor.document.state.project.clone();
    let undo = editor.history_label(Direction::Undo).map(str::to_owned);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert!(editor.active_pick().is_none());
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.history_label(Direction::Undo), undo.as_deref());
        });
        app.enter(|editor, cx| {
            editor.set_dmax(Some(DVec3::splat(0.005)));
            editor.sync_document();
            editor.begin_pick(Pick::Dmin, cx);
        });
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.dmin(), Some(expected));
            assert_eq!(
                editor.history_label(Direction::Undo),
                Some("Calibration pick")
            );
            assert!(editor.active_pick().is_none());
        });
    });
}
