use super::*;
use crate::{
    application::test_support::with_application,
    editor::{PickerCase, history::Direction},
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
        }
        .into(),
    );
    editor.document.source = false;
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

fn measured_reference(editor: &Editor, region: crate::coverage::Coverage) -> DVec3 {
    let reference = measurement::capture_scan(editor)
        .unwrap()
        .reference_coverage(region)
        .unwrap();
    let actual = query::reference(
        editor.source_services.as_ref().unwrap(),
        &mut measurement::capture_scan(editor).unwrap(),
        None,
        region,
        &|| false,
    )
    .unwrap();
    assert!((actual - reference).abs().max_element() / reference.abs().max_element() < 2e-6);
    actual
}

#[test]
fn each_reference_feature_awaits_the_same_camera_measurement_and_changes_only_its_endpoint() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    for kind in [PickerCase::Dmin, PickerCase::Dmax] {
        let editor = setup(file.path().to_str().unwrap());
        let region = patch(&editor);
        let expected = measured_reference(&editor, region);
        assert!(expected.min_element() > 0.005 && expected.max_element() < 0.2);
        let before = editor.document.state.project.clone();
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.begin_test_picker(kind, cx));
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.document.state.project, before);
                assert!(!editor.interaction_blocked());
            });
            app.enter(|editor, cx| {
                editor.set_test_pick_region(Some(region));
                editor.apply_pick(cx);
            });
            app.run_until(Duration::from_secs(5), |editor| {
                !editor.interaction_blocked()
            });
            app.read(|editor| {
                let calibration = editor.document.project.input.calibration().unwrap();
                assert_eq!(
                    calibration.dmin,
                    if kind == PickerCase::Dmin {
                        expected
                    } else {
                        DVec3::splat(0.2)
                    }
                );
                assert_eq!(
                    calibration.dmax,
                    if kind == PickerCase::Dmax {
                        expected
                    } else {
                        DVec3::splat(0.005)
                    }
                );
                assert_eq!(
                    editor.history_label(Direction::Undo),
                    Some("Calibration pick")
                );
                assert!(editor.test_pick().is_none());
                assert!(!editor.document.source);
            });
        });
    }
}

#[test]
fn roll_balance_picker_uses_one_patch_and_commits_both_balances_together() {
    let file = crate::raw::fixture::bayer_dng_with_as_shot_samples([1; 3], |x, _| {
        if x < 24 {
            [350, 600, 450]
        } else {
            [500, 470, 380]
        }
    });
    let mut editor = setup(file.path().to_str().unwrap());
    let region = patch(&editor);
    let camera = measured_reference(&editor, region);
    let spans = DVec3::new(0.8, 1., 1.25);
    let dmin = camera * spans.exp();
    let dmax = dmin * (-DVec3::new(1.42, 1.7, 2.06)).exp();
    let scan = measurement::capture_scan(&editor).unwrap();
    editor.load_input(Calibration { dmin, dmax }.into());
    editor.document.project.roll.exposure = 0.5;
    editor.document.project.roll.wb.warmth = 0.2;
    editor.photo_settings_mut().wb.tint = 0.2;
    let before = editor.document.state.project.clone();
    let input = editor.input();

    let mut edit = editor.edit_project("Dmin").unwrap();
    let changed = dmin * DVec3::new(1.1, 0.9, 1.);
    edit.update(|project| project.set_dmin(Some(changed.to_array())));
    editor.commit_project(edit).unwrap();

    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Gamma, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.input(), input);
            assert_eq!(editor.test_pick(), Some(GAMMA));
            assert_eq!(editor.test_pick_region(), Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        let after = app.read(|editor| {
            let negative = editor.input().negative_input().unwrap();
            let gamma = input.negative_input().unwrap().gamma;
            assert!(
                (negative.channel_gammas().unwrap() - spans * gamma)
                    .abs()
                    .max_element()
                    < 1e-12
            );
            let mut expected = before.as_ref().clone();
            let InputSettings::Negative { balance, .. } = &mut expected.input else {
                unreachable!();
            };
            *balance = negative.balance;
            let pipeline =
                crate::processing::InputPipeline::new(negative, scan.camera_to_ap1).unwrap();
            let positive = crate::sampling::weighted_mean_iter(
                region.pixels(scan.area).unwrap().map(|([x, y], weight)| {
                    let rgb = pipeline.apply(scan.pixel(x, y)?)?.rgb;
                    Ok((rgb, weight))
                }),
            )
            .unwrap();
            let wb = editor.document.project.roll.wb;
            let matrix = crate::color::wb_matrix(wb).unwrap();
            let neutral = matrix * positive;
            assert!((neutral / neutral.y - DVec3::ONE).abs().max_element() < 2e-6);
            let inverted_mean = matrix * pipeline.apply(camera).unwrap().rgb;
            assert!(
                (inverted_mean / inverted_mean.y - DVec3::ONE)
                    .abs()
                    .max_element()
                    > 1e-3
            );
            assert_ne!(wb, before.roll.wb);
            expected.roll.wb = wb;
            assert_eq!(*editor.document.state.project, expected);
            assert!(editor.status.contains("inferred Dmax"));
            assert_eq!(
                editor.history_label(Direction::Undo),
                Some("Balance roll from patch")
            );
            editor.document.state.project.clone()
        });
        app.enter(|editor, cx| {
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.document.state.project, before);
            editor.restore_history(Direction::Redo, cx);
            assert_eq!(editor.document.state.project, after);
        });
    });
}

#[test]
fn gamma_span_rejects_samples_at_film_base() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = setup(file.path().to_str().unwrap());
    let region = patch(&editor);
    let dmin = measured_reference(&editor, region) * 0.999;
    editor.load_input(
        Calibration {
            dmin,
            dmax: dmin * 0.2,
        }
        .into(),
    );
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Gamma, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(
                editor.status.contains("denser than film base"),
                "{}",
                editor.status
            );
        });
    });
}

#[test]
fn roll_balance_rejection_preserves_both_settings() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = setup(file.path().to_str().unwrap());
    let region = patch(&editor);
    let camera = measured_reference(&editor, region);
    let spans = DVec3::new(0.8, 1., 1.25);
    let dmin = camera * spans.exp();
    // Valid patch spans and calibration, but this anchor makes the patch too far
    // from neutral for the allowed Roll WB after installing those span ratios.
    let dmax = dmin * (-DVec3::new(0.8, 2., 4.)).exp();
    editor.load_input(Calibration { dmin, dmax }.into());
    editor.document.project.roll.wb.warmth = 0.2;
    let before = editor.document.state.project.clone();
    let undo = editor.history_label(Direction::Undo).map(str::to_owned);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Gamma, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert!(editor.status.contains("WB"), "{}", editor.status);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.history_label(Direction::Undo), undo.as_deref());
        });
    });
}

#[test]
fn inferred_dmax_keeps_the_previous_span_depth() {
    let base = DVec3::new(0.25, 0.35, 0.4);
    let old_spans = DVec3::new(1., 2., 4.);
    let spans = DVec3::new(0.8, 1., 1.25);
    let patch = base * (-spans).exp();
    let balance = crate::roll_analysis::balance_from_anchors(Calibration {
        dmin: base,
        dmax: patch,
    })
    .unwrap();
    let inferred = query::inferred_dmax(base, base * (-old_spans).exp(), balance).unwrap();
    assert!((inferred - base * (-2. * spans).exp()).abs().max_element() < 1e-12);
}

#[test]
fn incompatible_measured_reference_is_retained_and_undoable_until_its_companion_is_replaced() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = setup(file.path().to_str().unwrap());
    editor.load_input(
        Calibration {
            dmin: DVec3::splat(2.),
            dmax: DVec3::ONE,
        }
        .into(),
    );
    let region = patch(&editor);
    let expected = measured_reference(&editor, region);
    assert!(expected.min_element() > 0.005 && expected.max_element() < 1.);
    let before = editor.document.state.project.clone();
    let undo = editor.history_label(Direction::Undo).map(str::to_owned);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Dmin, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert!(editor.test_pick().is_none());
            assert_eq!(editor.dmin(), Some(expected));
            assert_eq!(editor.dmax(), Some(DVec3::ONE));
            assert_eq!(
                editor.history_label(Direction::Undo),
                Some("Calibration pick")
            );
            assert!(
                editor
                    .positive_processing(
                        editor.input(),
                        editor.document.project.roll,
                        editor.photo_settings()
                    )
                    .is_err()
            );
        });
        app.enter(|editor, cx| {
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.history_label(Direction::Undo), undo.as_deref());
            editor.restore_history(Direction::Redo, cx);
            assert_eq!(editor.dmin(), Some(expected));
            set_dmax(editor, DVec3::splat(0.005)).unwrap();
        });
        app.read(|editor| {
            assert_eq!(editor.dmin(), Some(expected));
            assert_eq!(editor.dmax(), Some(DVec3::splat(0.005)));
            editor
                .positive_processing(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
        });
    });
}
