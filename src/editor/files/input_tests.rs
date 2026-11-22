use super::*;
use crate::application::test_support::with_application;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(20);

#[test]
fn guarded_transition_reserves_input_before_poll_and_releases_an_abandoned_continuation() {
    let editor = crate::editor::tests::fixture();
    let accepted = editor.document.state.project.clone();
    assert!(editor.document.dirty());
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.guard_transition(Transition::Close, cx);
            assert!(editor.document.files.pending());
            assert!(!editor.dialogs.is_open());
            assert!(editor.workspace_input.is_blocked());
            assert!(!editor.tool_start_available());
            editor.document.files.action = None;
        });
        app.pump();
        app.read(|editor| {
            assert!(!editor.workspace_input.is_blocked());
            assert!(!editor.dialogs.is_open() && !editor.allow_close);
            assert_eq!(editor.document.state.project, accepted);
            assert!(editor.tool_start_available());
        });

        app.enter(|editor, cx| editor.guard_transition(Transition::Close, cx));
        app.run_until(LIMIT, |editor| editor.dialogs.is_open());
        app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
        app.run_until(LIMIT, |editor| !editor.document.files.pending());
        app.read(|editor| {
            assert!(!editor.workspace_input.is_blocked());
            assert!(!editor.allow_close);
            assert_eq!(editor.document.state.project, accepted);
        });
    });
}

#[test]
fn document_replacement_cancels_close_before_poll_while_waiting_and_after_a_ready_answer() {
    for phase in 0..3 {
        let editor = crate::editor::tests::fixture();
        assert!(editor.document.dirty());
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.guard_transition(Transition::Close, cx));
            if phase > 0 {
                app.run_until(LIMIT, |editor| editor.dialogs.is_open());
            }
            if phase == 2 {
                app.update(|editor| editor.dialogs.answer_for_test("Discard changes"));
            }
            app.update(Editor::new_project);
            app.pump();
            app.read(|editor| {
                assert!(!editor.allow_close);
                assert!(!editor.document.files.pending());
                assert!(editor.document.project.order.is_empty());
                assert_eq!(editor.dialogs.is_open(), phase == 1);
                assert_eq!(editor.workspace_input.is_blocked(), phase == 1);
            });
            if phase == 1 {
                app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
                app.pump();
                app.read(|editor| {
                    assert!(!editor.allow_close);
                    assert!(!editor.workspace_input.is_blocked());
                    assert!(editor.document.project.order.is_empty());
                });
            }
        });
    }
}

#[test]
fn failed_save_retains_input_across_the_notice_and_dirty_transition_reprompt() {
    let editor = crate::editor::tests::fixture();
    let accepted = editor.document.state.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("missing-parent")
        .join("project.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(
                path.clone(),
                crate::project::Save::New,
                Some(Transition::New),
                cx,
            );
            assert!(editor.workspace_input.is_blocked());
        });
        app.run_until(LIMIT, |editor| {
            !editor.document.write.pending() && editor.dialogs.is_open()
        });
        app.read(|editor| {
            assert!(editor.document.files.pending());
            assert!(editor.workspace_input.is_blocked());
            assert!(!editor.tool_start_available());
            assert_eq!(editor.document.state.project, accepted);
        });
        app.update(|editor| editor.dialogs.answer_for_test("OK"));
        app.pump();
        app.read(|editor| {
            assert!(editor.document.files.pending() && editor.dialogs.is_open());
            assert!(editor.workspace_input.is_blocked());
        });
        app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
        app.run_until(LIMIT, |editor| !editor.document.files.pending());
        app.read(|editor| {
            assert!(!editor.workspace_input.is_blocked());
            assert!(editor.tool_start_available());
            assert_eq!(editor.document.state.project, accepted);
        });
        assert!(!path.exists());
    });
}

#[test]
fn a_completed_save_keeps_the_transition_reserved_until_an_unread_notice_is_answered() {
    let editor = crate::editor::tests::fixture();
    let accepted = editor.document.state.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(
                path.clone(),
                crate::project::Save::New,
                Some(Transition::Close),
                cx,
            );
            editor.report_failure(
                crate::editor::tests::FailureCase::Export,
                "Independent notice".into(),
            );
        });
        app.run_until(LIMIT, |editor| !editor.document.write.pending());
        app.read(|editor| {
            assert!(editor.document.files.pending() && editor.dialogs.is_open());
            assert!(editor.workspace_input.is_blocked());
            assert!(!editor.tool_start_available());
            assert!(!editor.allow_close);
            assert!(!editor.document.dirty());
        });
        assert_eq!(crate::project::Project::load(&path).unwrap(), accepted);

        app.update(|editor| editor.dialogs.answer_for_test("OK"));
        app.run_until(LIMIT, |editor| {
            editor.allow_close && !editor.document.files.pending()
        });
        app.read(|editor| {
            assert!(!editor.workspace_input.is_blocked());
            assert_eq!(editor.document.state.project, accepted);
        });
    });
}
