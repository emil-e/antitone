use super::*;

fn frame(
    editor: &mut impl crate::editor::tests::UiFixture,
    context: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
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
fn system_quit_joins_one_dirty_decision_and_cancel_retires_it_without_mutation() {
    let (mut editor, context) = initialized();
    let before = editor.document.state.project.clone();
    assert!(editor.document.dirty());
    for _ in 0..3 {
        host::App::request_quit(&mut editor);
    }
    assert_eq!(
        editor
            .menus
            .pending
            .iter()
            .filter(|&&action| action == Action::Quit)
            .count(),
        1
    );
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Pending)
    );
    frame(&mut editor, &context, vec![]);
    assert_eq!(editor.transition, Some(Transition::Close));
    host::App::request_quit(&mut editor);
    assert!(
        editor.menus.pending.is_empty(),
        "join the existing decision, not another request"
    );
    click(&mut editor, &context, "Cancel");
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Running)
    );
    assert!(!editor.allow_close);
    assert_eq!(editor.document.state.project, before);
    assert!(
        editor.document_pending.is_none() && editor.document.export_operation.pending.is_none()
    );
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
    assert!(editor.document_pending.is_none());
}

#[test]
fn blocked_system_quit_never_accepts_a_tool_or_replaces_an_existing_decision() {
    for gate in 0..4 {
        let (mut editor, context) = initialized();
        match gate {
            0 => editor.busy = true,
            1 => editor.document.picking.kind = Some(Pick::Dmin),
            2 => editor.transition = Some(Transition::New),
            3 => editor.report_failure(FailureKind::Save, "fixture failure".into()),
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
        assert_eq!(
            editor.document.picking.kind,
            (gate == 1).then_some(Pick::Dmin)
        );
        assert_eq!(editor.transition, (gate == 2).then_some(Transition::New));
        assert!(
            editor.document_pending.is_none() && editor.document.export_operation.pending.is_none()
        );
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
        editor.document.roll.exposure = 0.25;
        editor.sync_document();
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.update(host::App::request_quit);
            frame(app, &context, vec![]);
            click(app, &context, "Save changes");
            let request = app.read(|e| {
                assert_eq!(host::App::quit_state(e), Some(host::QuitState::Pending));
                assert!(!e.allow_close);
                e.document_pending.unwrap()
            });
            app.enter(|e, cx| e.save_result(request + 1, Err("stale save failure".into()), cx));
            app.read(|e| {
                assert_eq!(host::App::quit_state(e), Some(host::QuitState::Pending));
                assert_eq!(e.document_pending, Some(request));
            });
            app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
            if success {
                app.read(|e| {
                    assert_eq!(host::App::quit_state(e), Some(host::QuitState::Approved));
                    assert!(!e.document.dirty() && e.failure.is_none());
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
                    assert_eq!(host::App::quit_state(e), Some(host::QuitState::Running));
                    assert!(!e.allow_close && e.document.dirty());
                    assert_eq!(e.document.state.project, before);
                    assert!(e.failure.is_some());
                });
                assert!(path.is_dir());
            }
        });
    }
}

#[test]
fn cancelling_project_replacement_retires_the_native_close_continuation() {
    let (mut editor, context) = initialized();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing.antitone");
    std::fs::write(&destination, b"existing project").unwrap();
    editor.project_overwrite = Some((destination.clone(), Some(Transition::Close)));
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Pending)
    );
    click(&mut editor, &context, "Cancel");
    assert_eq!(
        host::App::quit_state(&editor),
        Some(host::QuitState::Running)
    );
    assert!(!editor.allow_close);
    assert_eq!(std::fs::read(destination).unwrap(), b"existing project");
    assert!(
        editor.document_pending.is_none() && editor.document.export_operation.pending.is_none()
    );
}
