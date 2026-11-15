use super::*;
use crate::editor::{Transition, files::FileAction};
use crate::platform::menu::Action;

fn save_baseline(editor: &mut Editor, path: &std::path::Path) {
    *editor.document = editor
        .document
        .capture()
        .save(path, crate::project::Save::New)
        .unwrap();
    assert!(!editor.document.dirty());
}

#[test]
fn successful_new_or_open_retires_a_running_search_without_accepting_or_waiting_for_it() {
    for open in [false, true] {
        let mut editor = editor();
        let directory = tempfile::tempdir().unwrap();
        save_baseline(&mut editor, &directory.path().join("old.antitone"));
        let next = directory.path().join("next.antitone");
        let mut target = crate::project::Project::default();
        target.roll.exposure = 0.3;
        target.save(&next, crate::project::Save::New).unwrap();
        with_search(editor, |app, gate| {
            app.enter(|editor, cx| {
                queue(editor, [80, 80], cx);
            });
            let call = gate.next(app);
            assert!(!call.cancelled.is_cancelled());
            app.enter(|editor, cx| {
                assert!(editor.dusting() && editor.dust_search_pending());
                assert!(!editor.editing() && !editor.workspace_input.is_blocked());
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
            app.run_until(Duration::from_secs(15), |editor| !editor.files.pending());
            app.run_until(Duration::from_secs(5), |_| call.cancelled.is_cancelled());
            let (accepted, status) = app.read(|editor| {
                assert!(!editor.dusting() && !editor.dust_search_pending());
                assert!(editor.document.project.sources.is_empty());
                assert_eq!(
                    editor.document.project.roll.exposure,
                    if open { 0.3 } else { 0. }
                );
                (editor.document.state.project.clone(), editor.status.clone())
            });
            assert!(
                call.input.context.scan().is_ok(),
                "captured physical input remains usable through cleanup"
            );
            call.complete(Err("late retired search failure".into()));
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.status, status);
                assert!(!editor.dialogs.is_open());
                assert_eq!(editor.history_label(history::Direction::Undo), None);
            });
        });
    }
}

#[test]
fn cancelled_departure_or_failed_open_preserves_the_search_and_it_can_still_accept() {
    for fail_open in [false, true] {
        let mut editor = editor();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.antitone");
        save_baseline(&mut editor, &path);
        editor.document.project.roll.exposure = 0.5;
        let before = editor.document.state.project.clone();
        let count = editor
            .dust_repairs(editor.document.visit.current_source.unwrap())
            .unwrap()
            .repairs()
            .len();
        with_search(editor, |app, gate| {
            let stroke = app.enter(|editor, cx| queue(editor, [80, 80], cx));
            let call = gate.next(app);
            app.enter(|editor, cx| {
                if fail_open {
                    assert!(editor.application_menu_state().enabled(Action::Open));
                    editor.guard_transition(
                        Transition::Open(directory.path().join("missing.antitone")),
                        cx,
                    );
                } else {
                    editor.file_action(FileAction::New, cx);
                }
            });
            app.run_until(Duration::from_secs(15), |editor| editor.dialogs.is_open());
            app.read(|editor| assert!(editor.dusting() && editor.dust_search_pending()));
            app.update(|editor| {
                editor.dialogs.answer_for_test(if fail_open {
                    "Discard changes"
                } else {
                    "Cancel"
                })
            });
            app.run_until(Duration::from_secs(15), |editor| !editor.files.pending());
            assert!(!call.cancelled.is_cancelled());
            app.read(|editor| {
                assert_eq!(editor.document.path.as_ref(), Some(&path));
                assert_eq!(editor.document.state.project, before);
                assert!(editor.dusting() && editor.dust_search_pending());
            });
            call.complete(Ok(SearchOutcome::Candidate([20, 0])));
            settled(app);
            app.read(|editor| {
                let repairs = editor.dust_repairs(stroke.source).unwrap().repairs();
                assert_eq!(repairs.len(), count + 1);
                assert_eq!(repairs.last().unwrap().support, stroke.support);
                assert_eq!(repairs.last().unwrap().offset, [20, 0]);
                assert!(editor.document.dirty());
            });
        });
    }
}

#[test]
fn close_approves_before_search_cleanup_and_application_retirement_withdraws_publication() {
    let mut editor = editor();
    let directory = tempfile::tempdir().unwrap();
    save_baseline(&mut editor, &directory.path().join("old.antitone"));
    let before = editor.document.state.project.clone();
    with_search(editor, |app, gate| {
        app.enter(|editor, cx| {
            queue(editor, [80, 80], cx);
        });
        let call = gate.next(app);
        app.enter(|editor, cx| {
            for action in [Action::Close, Action::Quit] {
                assert!(editor.application_menu_state().enabled(action));
            }
            editor.file_action(FileAction::Close, cx);
            assert!(editor.allow_close && !editor.files.pending());
            assert!(!editor.document.write.pending());
        });
        app.close_execution();
        assert!(call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        app.pump();
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(!editor.dialogs.is_open());
        });
    });
}
