use super::*;
use crate::application::test_support::with_application;
use std::time::Duration;

#[test]
fn removal_cancel_keeps_the_crop_and_confirmation_retires_it_without_applying() {
    let mut editor = fixture(2);
    editor.begin_crop();
    let draft = Crop::from_region(Rect {
        x: 100,
        y: 80,
        width: 320,
        height: 240,
    });
    editor.document.crop_edit_mut().unwrap().draft = draft;
    let view = editor.document.visit.view;
    let removed = editor.document.visit.current_source.unwrap();
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_removal(cx));
        app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
        app.run_until(Duration::from_secs(5), |editor| !editor.dialogs.is_open());
        app.pump();
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.document.visit.view, view);
            assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
        });
        app.enter(|editor, cx| editor.request_removal(cx));
        app.update(|editor| {
            let mut edit = editor.edit_project("Independent exposure").unwrap();
            edit.update(|project| project.roll.exposure = 0.75);
            editor.commit_project(edit).unwrap();
            editor.export_failed("An unrelated notice remains queued".into());
            editor.dialogs.answer_for_test("Remove photos");
        });
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.project.source(removed).is_err()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.order.len(), before.order.len() - 1);
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            assert!(!editor.cropping());
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|source| source.crop.is_none())
            );
            assert!(
                editor.dialogs.is_open(),
                "confirmed mutation can publish under an unread notice"
            );
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Remove photos")
            );
        });
    });
}

#[test]
fn an_actual_edit_refuses_removal_then_release_admits_it() {
    let mut editor = fixture(1);
    let before = editor.document.state.project.clone();
    let edit = editor.edit_project("Held edit").unwrap();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_removal(cx));
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(!editor.dialogs.is_open());
            assert!(editor.editing());
        });
        drop(edit);
        app.enter(|editor, cx| editor.request_removal(cx));
        app.read(|editor| assert!(editor.dialogs.is_open()));
        app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
    });
}

#[test]
fn an_edit_acquired_after_confirmation_was_requested_is_not_cancelled_to_force_removal() {
    let editor = fixture(1);
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_removal(cx));
        let edit = app.update(|editor| editor.edit_project("Background edit").unwrap());
        app.update(|editor| editor.dialogs.answer_for_test("Remove photos"));
        app.pump();
        app.read(|editor| {
            assert!(editor.editing());
            assert_eq!(editor.document.state.project, before);
            assert!(!editor.dialogs.is_open());
        });
        drop(edit);
        app.enter(|editor, cx| editor.request_removal(cx));
        app.update(|editor| editor.dialogs.answer_for_test("Remove photos"));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.project.order.len() < before.order.len()
        });
    });
}
