use super::*;

fn frame(
    editor: &mut impl crate::editor::tests::UiFixture,
    context: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    editor.before_frame();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| editor.ui(ui),
    );
    output.textures_delta.clear();
    editor.before_frame();
    output
}

fn initialized() -> (Editor, egui::Context) {
    let mut editor = super::super::tests::fixture();
    let context = egui::Context::default();
    for _ in 0..3 {
        frame(&mut editor, &context, vec![]);
    }
    (editor, context)
}

fn click(editor: &mut impl crate::editor::tests::UiFixture, context: &egui::Context, label: &str) {
    frame(editor, context, vec![]);
    let output = frame(editor, context, vec![]);
    let point = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
            }
            _ => None,
        })
        .expect("dialog action is visible");
    for pressed in [true, false] {
        frame(
            editor,
            context,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn system_quit_coalesces_an_already_pending_host_action() {
    let (mut editor, _) = initialized();
    editor.menus.pending.push_back(Action::Quit);
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Pending)
    );
    host::App::request_quit(&mut editor);
    assert_eq!(editor.menus.pending.len(), 1);
    editor.menus.pending.clear();
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Running)
    );
}

#[test]
fn system_quit_joins_one_dirty_decision_and_cancel_retires_it_without_mutation() {
    let (mut editor, context) = initialized();
    let before = editor.document.project.clone();
    assert!(editor.document.dirty());
    for _ in 0..3 {
        host::App::request_quit(&mut editor);
    }
    assert_eq!(
        editor
            .menus
            .pending
            .iter()
            .filter(|&&a| a == Action::Quit)
            .count(),
        1
    );
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Pending)
    );
    crate::application::test_support::with_application(editor, |app| {
        frame(app, &context, vec![]);
        app.update(|editor| {
            assert!(editor.files.closing());
            host::App::request_quit(editor);
            assert!(
                editor.menus.pending.is_empty(),
                "join the existing decision"
            );
        });
        click(app, &context, "Cancel");
        app.read(|editor| {
            assert_eq!(
                host::App::quit_state(editor),
                Some(host::QuitState::Running)
            );
            assert!(!editor.allow_close);
            assert_eq!(editor.document.project, before);
            assert!(!editor.document.write.pending() && editor.document.export.operation.is_none());
        });
    });
}

#[test]
fn a_clean_system_quit_approves_without_saving_again() {
    let (mut editor, context) = initialized();
    let directory = tempfile::tempdir().unwrap();
    *editor.document = editor
        .document
        .save(
            &directory.path().join("clean.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    host::App::request_quit(&mut editor);
    frame(&mut editor, &context, vec![]);
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Approved)
    );
    assert!(editor.allow_close && !editor.document.dirty());
    assert!(!editor.document.write.pending());
}

#[test]
fn blocked_system_quit_never_accepts_a_tool_or_replaces_an_existing_decision() {
    for gate in 0..4 {
        let (mut editor, context) = initialized();
        match gate {
            0 => editor.set_test_busy(true),
            1 => {
                super::super::tests::add_ready_fixture(&mut editor);
                editor.begin_crop();
                assert!(editor.cropping());
            }
            2 => editor.block_with_decision(),
            3 => editor.report_failure(FailureCase::Save, "fixture failure".into()),
            _ => unreachable!(),
        }
        let before = editor.document.state.project.clone();
        host::App::request_quit(&mut editor);
        frame(&mut editor, &context, vec![]);
        assert_eq!(
            host::App::quit_state(&editor),
            Some(host::QuitState::Running)
        );
        assert!(!editor.allow_close && editor.menus.pending.is_empty());
        assert_eq!(editor.document.state.project, before);
        assert_eq!(editor.cropping(), gate == 1);
        assert_eq!(editor.dialogs.is_open(), gate == 2 || gate == 3);
        assert!(!editor.document.write.pending() && editor.document.export.operation.is_none());
    }
}

#[test]
fn system_quit_waits_for_save_success_and_cannot_approve_a_failed_or_stale_save() {
    for success in [false, true] {
        let (editor, context) = initialized();
        let mut editor = super::super::tests::with_document_services(editor);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.antitone");
        *editor.document = editor
            .document
            .save(&path, crate::project::Save::New)
            .unwrap();
        if !success {
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
        }
        editor.document.project.roll.exposure = 0.25;

        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.update(host::App::request_quit);
            frame(app, &context, vec![]);
            click(app, &context, "Save changes");
            app.read(|e| {
                assert_eq!(host::App::quit_state(e), Some(host::QuitState::Pending));
                assert!(!e.allow_close && e.document.write.pending());
            });
            app.run_until(Duration::from_secs(15), |e| !e.document.write.pending());
            if success {
                app.read(|e| {
                    assert_eq!(host::App::quit_state(e), Some(host::QuitState::Approved));
                    assert!(!e.document.dirty() && !e.dialogs.is_open());
                });
                assert_eq!(crate::project::Project::load(&path).unwrap(), before);
                let presented = frame(app, &context, vec![]);
                assert!(presented.viewport_output.values().any(|viewport| {
                    viewport
                        .commands
                        .iter()
                        .any(|command| matches!(command, egui::ViewportCommand::Close))
                }));
            } else {
                app.read(|e| {
                    assert_eq!(host::App::quit_state(e), Some(host::QuitState::Pending));
                    assert!(!e.allow_close && e.document.dirty());
                    assert_eq!(e.document.state.project, before);
                    assert!(e.dialogs.is_open());
                });
                assert!(path.is_dir());
            }
        });
    }
}

#[test]
fn cancelling_project_replacement_retires_the_native_close_continuation() {
    let (editor, context) = initialized();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing.antitone");
    std::fs::write(&destination, b"existing project").unwrap();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_destination_action(
                super::super::files::Destination {
                    path: destination.clone(),
                    policy: crate::project::Save::Replace,
                    confirm_replace: true,
                },
                Some(Transition::Close),
                cx,
            )
        });
        assert_eq!(
            app.read(host::App::quit_state),
            Some(host::QuitState::Pending)
        );
        click(app, &context, "Cancel");
        app.read(|editor| {
            assert_eq!(
                host::App::quit_state(editor),
                Some(host::QuitState::Running)
            );
            assert!(!editor.allow_close);
            assert!(!editor.document.write.pending() && editor.document.export.operation.is_none());
        });
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing project");
    });
}
