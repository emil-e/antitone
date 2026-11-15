use super::*;
use crate::{
    application::test_support::with_application,
    editor::{Transition, files::FileAction},
    platform::menu::Action,
};
use std::time::Duration;
use workflow_tests::pending;

// Retired physical work has no surviving UI continuation to notify the application.
fn wait_for_input_release(
    app: &mut crate::application::test_support::TestApplication<'_, Editor>,
    input: &std::sync::Weak<crate::looks::Lut3D>,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while input.upgrade().is_some() {
        app.pump();
        assert!(
            std::time::Instant::now() < deadline,
            "retired preparation retained its input"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn successful_new_or_open_retires_pending_publication_while_physical_inputs_survive_cleanup() {
    for open in [false, true] {
        let (mut editor, scheduler) = tests::editor();
        let directory = tempfile::tempdir().unwrap();
        *editor.document = editor
            .document
            .save(
                &directory.path().join("old.antitone"),
                crate::project::Save::New,
            )
            .unwrap();
        let next = directory.path().join("next.antitone");
        let mut project = crate::project::Project::default();
        project.roll.exposure = 0.3;
        project.save(&next, crate::project::Save::New).unwrap();
        let selected = tests::asset(0.7);
        let pixels = Arc::downgrade(selected.lut());
        with_application(editor, |app| {
            let call = pending(app, Slot::Lut, Source::Loaded(selected));
            app.enter(|editor, cx| {
                assert!(editor.application_menu_state().enabled(if open {
                    Action::Open
                } else {
                    Action::New
                }));
                if open {
                    editor.guard_transition(Transition::Open(next.clone()), cx);
                } else {
                    editor.file_action(FileAction::New, cx);
                }
            });
            app.run_until(Duration::from_secs(15), |editor| !editor.files.opening());
            app.read(|editor| {
                assert!(editor.document.lut.pending.is_none());
                assert!(editor.document.project.output.lut.is_none());
                assert_eq!(
                    editor.document.project.roll.exposure,
                    if open { 0.3 } else { 0. }
                );
            });
            assert!(call.cancelled.is_cancelled());
            assert!(
                pixels.upgrade().is_some(),
                "physical work owns its prepared input"
            );
            call.finish();
            wait_for_input_release(app, &pixels);
            app.read(|editor| assert!(editor.document.project.output.lut.is_none()));
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn failed_open_preserves_the_pending_request_and_its_original_document() {
    let (mut editor, scheduler) = tests::editor();
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.antitone");
    *editor.document = editor
        .document
        .save(&old, crate::project::Save::New)
        .unwrap();
    let chosen = tests::asset(0.7);
    with_application(editor, |app| {
        let call = pending(app, Slot::Lut, Source::Loaded(chosen.clone()));
        let operation = app.read(|editor| {
            editor
                .document
                .lut
                .pending
                .as_ref()
                .unwrap()
                .activity
                .clone()
        });
        app.enter(|editor, cx| {
            editor.guard_transition(
                Transition::Open(directory.path().join("missing.antitone")),
                cx,
            )
        });
        app.run_until(Duration::from_secs(15), |editor| !editor.files.opening());
        app.read(|editor| {
            assert_eq!(editor.document.path.as_ref(), Some(&old));
            assert_eq!(
                editor.document.lut.pending.as_ref().unwrap().activity,
                operation
            );
        });
        assert!(!call.cancelled.is_cancelled());
        call.finish();
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.lut.pending.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.path.as_ref(), Some(&old));
            assert_eq!(editor.document.project.output.lut, Some(chosen.clone()));
            assert!(editor.document.dirty());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_a_dirty_transition_preserves_pending_or_newly_accepted_choice_under_its_question() {
    for finish_under_question in [false, true] {
        let (mut editor, scheduler) = tests::editor();
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("old.antitone");
        *editor.document = editor
            .document
            .save(&old, crate::project::Save::New)
            .unwrap();
        editor.document.project.roll.exposure = 0.5;
        let chosen = tests::asset(0.7);
        with_application(editor, |app| {
            let call = pending(app, Slot::Lut, Source::Loaded(chosen.clone()));
            app.enter(|editor, cx| editor.file_action(FileAction::New, cx));
            app.run_until(Duration::from_secs(15), |editor| editor.dialogs.is_open());
            let call = if finish_under_question {
                call.finish();
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.lut.pending.is_none()
                });
                app.read(|editor| assert!(editor.dialogs.is_open() && editor.files.pending()));
                None
            } else {
                Some(call)
            };
            app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
            app.run_until(Duration::from_secs(15), |editor| !editor.files.pending());
            if let Some(call) = call {
                assert!(!call.cancelled.is_cancelled());
                call.finish();
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.lut.pending.is_none()
                });
            }
            app.read(|editor| {
                assert_eq!(editor.document.path.as_ref(), Some(&old));
                assert_eq!(editor.document.project.roll.exposure, 0.5);
                assert_eq!(editor.document.project.output.lut, Some(chosen.clone()));
                assert!(!editor.dialogs.is_open());
                assert!(editor.document.dirty());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn normal_close_is_admitted_and_application_retirement_withdraws_pending_publication() {
    let (mut editor, scheduler) = tests::editor();
    let directory = tempfile::tempdir().unwrap();
    *editor.document = editor
        .document
        .save(
            &directory.path().join("old.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    let chosen = tests::asset(0.7);
    let pixels = Arc::downgrade(chosen.lut());
    with_application(editor, |app| {
        let call = pending(app, Slot::Lut, Source::Loaded(chosen));
        app.enter(|editor, cx| {
            for action in [Action::Close, Action::Quit] {
                assert!(editor.application_menu_state().enabled(action));
            }
            editor.guard_transition(Transition::Close, cx);
            assert!(editor.allow_close);
        });
        app.close_execution();
        assert!(call.cancelled.is_cancelled());
        call.finish();
        wait_for_input_release(app, &pixels);
        app.read(|editor| assert!(editor.document.project.output.lut.is_none()));
    });
    scheduler.close_and_wait().unwrap();
}
