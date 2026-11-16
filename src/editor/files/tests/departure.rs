use super::*;
use crate::{application::test_support::TestApplication, platform::menu::Action};
use std::time::Duration;

fn ready() -> Editor {
    let mut editor = fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor
}

fn save_baseline(editor: &mut Editor, path: &std::path::Path) {
    *editor.document = editor
        .document
        .capture()
        .save(path, crate::project::Save::New)
        .unwrap();
    assert!(!editor.document.dirty());
}

fn draft_crop(editor: &mut Editor) -> Crop {
    editor.begin_crop();
    let draft = Crop::from_region(Rect {
        x: 100,
        y: 80,
        width: 320,
        height: 240,
    });
    editor.document.crop_edit_mut().unwrap().draft = draft;
    assert!(!editor.editing());
    draft
}

fn settle(app: &mut TestApplication<'_, Editor>) {
    app.run_until(Duration::from_secs(15), |editor| !editor.files.pending());
}

#[test]
fn clean_new_and_close_accept_unapplied_crop_or_armed_mask_without_a_draft_loss_prompt() {
    for mask in [false, true] {
        for action in [FileAction::New, FileAction::Close] {
            let mut editor = ready();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("saved.antitone");
            save_baseline(&mut editor, &path);
            let accepted = editor.document.state.project.clone();
            if mask {
                editor.begin_radial_mask(None);
                assert!(editor.creating_mask());
            } else {
                draft_crop(&mut editor);
            }
            with_application(editor, |app| {
                app.enter(|editor, cx| {
                    assert!(!editor.document.dirty());
                    for command in [Action::New, Action::Open, Action::Close, Action::Quit] {
                        assert!(editor.application_menu_state().enabled(command));
                    }
                    assert!(editor.application_menu_state().enabled(Action::AddScans));
                    editor.file_action(action, cx);
                });
                settle(app);
                app.read(|editor| {
                    assert!(!editor.dialogs.is_open() && !editor.document.write.pending());
                    if action == FileAction::New {
                        assert!(editor.document.project.sources.is_empty());
                        assert!(!editor.has_hosted_tool());
                        assert!(!editor.document.dirty());
                    } else {
                        assert!(editor.allow_close);
                        assert_eq!(editor.document.state.project, accepted);
                    }
                });
            });
            assert_eq!(crate::project::Project::load(&path).unwrap(), accepted);
        }
    }
}

#[test]
fn dirty_departure_keeps_save_discard_cancel_and_never_applies_the_crop_proposal() {
    for action in [FileAction::New, FileAction::Close] {
        for answer in ["Save changes", "Discard changes", "Cancel"] {
            let mut editor = ready();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("saved.antitone");
            save_baseline(&mut editor, &path);
            let saved = editor.document.state.project.clone();
            editor.document.project.roll.exposure = 0.75;
            let accepted = editor.document.state.project.clone();
            let draft = draft_crop(&mut editor);
            with_application(editor, |app| {
                app.enter(|editor, cx| editor.file_action(action, cx));
                app.run_until(Duration::from_secs(15), |editor| editor.dialogs.is_open());
                app.read(|editor| {
                    assert!(!editor.allow_close);
                    assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                    assert_eq!(editor.document.state.project, accepted);
                });
                app.update(|editor| editor.dialogs.answer_for_test(answer));
                settle(app);
                app.read(|editor| {
                    if answer == "Cancel" {
                        assert!(!editor.allow_close);
                        assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                        assert_eq!(editor.document.state.project, accepted);
                        assert!(editor.document.dirty());
                    } else if action == FileAction::New {
                        assert!(!editor.has_hosted_tool());
                        assert!(editor.document.project.sources.is_empty());
                    } else {
                        assert!(editor.allow_close);
                        assert_eq!(editor.document.state.project, accepted);
                    }
                    assert!(!editor.dialogs.is_open());
                });
            });
            assert_eq!(
                crate::project::Project::load(&path).unwrap(),
                if answer == "Save changes" {
                    accepted
                } else {
                    saved
                }
            );
        }
    }
}

#[test]
fn failed_open_after_discard_preserves_the_original_document_and_crop_proposal() {
    for dirty in [false, true] {
        let mut editor = ready();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.antitone");
        save_baseline(&mut editor, &path);
        if dirty {
            editor.document.project.roll.exposure = 0.75;
        }
        let accepted = editor.document.state.project.clone();
        let draft = draft_crop(&mut editor);
        let view = editor.document.visit.view;
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                assert!(editor.application_menu_state().enabled(Action::Open));
                // Supply the chooser result; the normal guard and reader still run.
                editor.guard_transition(
                    Transition::Open(directory.path().join("missing.antitone")),
                    cx,
                );
            });
            if dirty {
                app.run_until(Duration::from_secs(15), |editor| editor.dialogs.is_open());
                app.update(|editor| editor.dialogs.answer_for_test("Discard changes"));
            }
            settle(app);
            app.read(|editor| {
                assert_eq!(editor.document.path.as_ref(), Some(&path));
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.visit.view, view);
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert_eq!(editor.document.dirty(), dirty);
                assert!(!editor.allow_close);
                assert!(editor.status.starts_with("Project:"), "{}", editor.status);
            });
        });
    }
}

#[test]
fn native_fallback_and_viewport_departure_routes_keep_the_dirty_crop_until_the_decision() {
    for route in 0..5 {
        let editor = ready();
        let context = egui::Context::default();
        with_application(editor, |app| {
            ui_frame(app, &context, vec![]);
            let (accepted, draft) = app.update(|editor| {
                let accepted = editor.document.state.project.clone();
                let draft = draft_crop(editor);
                if route < 3 {
                    host::App::native_menu_installed(editor);
                    host::App::menu_action(
                        editor,
                        [Action::New, Action::Close, Action::Quit][route],
                    );
                }
                (accepted, draft)
            });
            if route == 3 {
                ui_frame(
                    app,
                    &context,
                    vec![egui::Event::Key {
                        key: egui::Key::N,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                );
            } else if route == 4 {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 800.),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .events
                    .push(egui::ViewportEvent::Close);
                let mut output = context.run_ui(input, |ui| {
                    egui::CentralPanel::default()
                        .show(ui, |ui| crate::editor::tests::paint_controller(app, ui));
                });
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&egui::ViewportCommand::CancelClose)
                );
                output.textures_delta.clear();
            } else {
                ui_frame(app, &context, vec![]);
            }
            app.run_until(Duration::from_secs(5), |editor| editor.dialogs.is_open());
            app.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert!(!editor.allow_close);
            });
            app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
            settle(app);
            app.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert_eq!(
                    host::App::quit_state(editor),
                    Some(host::QuitState::Running)
                );
            });
        });
    }
}

#[test]
fn failed_open_keeps_a_picker_waiting_but_successful_new_retires_its_continuation() {
    let mut editor = ready();
    let directory = tempfile::tempdir().unwrap();
    save_baseline(&mut editor, &directory.path().join("saved.antitone"));
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.begin_test_picker(crate::editor::tests::PickerCase::Dmin, cx)
        });
        app.run_until(Duration::from_secs(5), |editor| {
            editor.test_pick().is_some()
        });
        let display = app.read(Editor::test_pick);
        app.enter(|editor, cx| {
            assert!(editor.application_menu_state().enabled(Action::Open));
            editor.guard_transition(
                Transition::Open(directory.path().join("missing.antitone")),
                cx,
            );
        });
        settle(app);
        app.read(|editor| {
            assert_eq!(editor.test_pick(), display);
            assert_eq!(editor.document.state.project, before);
        });
        app.enter(|editor, cx| editor.file_action(FileAction::New, cx));
        settle(app);
        app.pump();
        app.read(|editor| {
            assert!(editor.test_pick().is_none());
            assert!(
                editor
                    .document
                    .visit
                    .interaction_action
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
            );
            assert!(editor.document.project.sources.is_empty());
            assert_eq!(editor.history_label(history::Direction::Undo), None);
        });
    });
}

#[test]
fn live_edit_and_input_reservation_reject_direct_departure_without_retiring_their_owners() {
    for reserved in [false, true] {
        let mut editor = ready();
        let before = editor.document.state.project.clone();
        let edit = (!reserved).then(|| editor.edit_project("Held edit").unwrap());
        let block = reserved.then(|| editor.workspace_input.block());
        with_application(editor, |app| {
            for action in [FileAction::New, FileAction::Open, FileAction::Close] {
                app.enter(|editor, cx| {
                    for command in [Action::New, Action::Open, Action::Close, Action::Quit] {
                        assert!(!editor.application_menu_state().enabled(command));
                    }
                    editor.file_action(action, cx);
                    assert_eq!(editor.document.state.project, before);
                    assert!(
                        !editor.files.pending() && !editor.dialogs.is_open() && !editor.allow_close
                    );
                    assert_eq!(editor.editing(), !reserved);
                    assert_eq!(editor.workspace_input.is_blocked(), reserved);
                });
            }
            drop(edit);
            drop(block);
            app.read(|editor| {
                assert!(editor.application_menu_state().enabled(Action::New));
            });
            app.enter(|editor, cx| editor.file_action(FileAction::New, cx));
            app.run_until(Duration::from_secs(15), |editor| editor.dialogs.is_open());
            app.update(|editor| editor.dialogs.answer_for_test("Discard changes"));
            settle(app);
            app.read(|editor| assert!(editor.document.project.sources.is_empty()));
        });
    }
}
