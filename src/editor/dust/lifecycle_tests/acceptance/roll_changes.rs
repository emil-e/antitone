use super::*;
use crate::editor::tests::{accepted_input, authored_fixture};
use crate::processing::InputSettings;

fn photos() -> (Editor, tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let first = directory.path().join("first.dng");
    let second = directory.path().join("second.dng");
    std::fs::copy(&root, &first).unwrap();
    std::fs::copy(&root, &second).unwrap();
    let editor = accepted_input(
        authored_fixture(&[first.to_str().unwrap()]),
        InputSettings::Digital,
    );
    (editor, directory, second)
}

#[test]
fn import_keeps_the_dust_owner_and_normal_edit_authority_decides_completion_conflicts() {
    for finish_first in [false, true] {
        let (editor, _directory, second) = photos();
        let source = editor.document.visit.current_source.unwrap();
        let view = editor.visit_lifetime();
        with_search(editor, |app, gate| {
            app.enter(|editor, cx| {
                queue(editor, [80, 80], cx);
            });
            let call = gate.next(app);
            if finish_first {
                call.complete(Ok(SearchOutcome::Candidate([20, 0])));
                settled(app);
                app.read(|editor| {
                    assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 1)
                });
                app.enter(|editor, cx| editor.add_raws(vec![second], cx));
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.importing.is_none()
                });
            } else {
                let paused = crate::background::testing::pause();
                app.enter(|editor, cx| {
                    editor.add_raws(vec![second], cx);
                    assert!(editor.editing() && editor.workspace_input.is_blocked());
                });
                call.complete(Ok(SearchOutcome::Candidate([20, 0])));
                settled(app);
                app.read(|editor| {
                    assert!(
                        editor.status.contains("finish or cancel the current edit"),
                        "{}",
                        editor.status
                    );
                    assert!(
                        editor
                            .document
                            .project
                            .source(source)
                            .unwrap()
                            .repairs
                            .repairs()
                            .is_empty()
                    );
                    assert!(editor.document.importing.is_some());
                });
                drop(paused);
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.importing.is_none()
                });
            }
            app.read(|editor| {
                assert!(editor.dusting());
                assert_eq!(editor.document.visit.current_source, Some(source));
                assert!(view.upgrade().is_some());
                assert_eq!(editor.document.project.order.len(), 2);
                assert_eq!(
                    editor
                        .document
                        .project
                        .source(source)
                        .unwrap()
                        .repairs
                        .repairs()
                        .len(),
                    usize::from(finish_first)
                );
            });
        });
    }
}

#[test]
fn search_held_through_import_can_publish_after_import_releases_edit_authority() {
    let (editor, _directory, second) = photos();
    let source = editor.document.visit.current_source.unwrap();
    let view = editor.visit_lifetime();
    with_search(editor, |app, gate| {
        app.enter(|editor, cx| {
            queue(editor, [80, 80], cx);
        });
        let call = gate.next(app);
        app.enter(|editor, cx| editor.add_raws(vec![second], cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.importing.is_none()
        });
        app.read(|editor| {
            assert!(editor.dusting() && editor.dust_search_pending());
            assert!(!call.cancelled.is_cancelled());
            assert!(!editor.editing());
            assert_eq!(editor.document.project.order.len(), 2);
        });
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|editor| {
            assert!(editor.dusting());
            assert!(view.upgrade().is_some());
            assert_eq!(editor.document.visit.current_source, Some(source));
            assert_eq!(editor.document.project.order.len(), 2);
            assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 1);
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Dust mask")
            );
        });
    });
}

#[test]
fn cancelled_removal_keeps_the_search_but_confirmation_retires_it_and_late_result() {
    let (editor, _directory, _) = photos();
    let source = editor.document.visit.current_source.unwrap();
    with_search(editor, |app, gate| {
        app.enter(|editor, cx| {
            queue(editor, [80, 80], cx);
        });
        let call = gate.next(app);
        let before = app.read(|editor| editor.document.state.project.clone());
        app.enter(|editor, cx| editor.request_removal(cx));
        app.read(|editor| assert!(editor.dialogs.is_open() && editor.dust_search_pending()));
        app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
        app.pump();
        assert!(!call.cancelled.is_cancelled());
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(editor.dusting() && editor.dust_search_pending());
        });
        app.enter(|editor, cx| editor.request_removal(cx));
        app.update(|editor| editor.dialogs.answer_for_test("Remove photos"));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.project.source(source).is_err()
        });
        app.run_until(Duration::from_secs(5), |_| call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        app.pump();
        app.read(|editor| {
            assert!(editor.document.project.sources.is_empty());
            assert!(!editor.dusting() && !editor.dust_search_pending());
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Remove photos")
            );
        });
    });
}
