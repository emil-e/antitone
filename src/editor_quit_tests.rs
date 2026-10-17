use super::*;

fn frame(
    editor: &mut Editor,
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
        |ui| host::App::ui(editor, ui),
    );
    output.textures_delta.clear();
    output
}

fn initialized() -> (
    Editor,
    egui::Context,
    mpsc::Sender<Message>,
    mpsc::Receiver<Command>,
) {
    let (mut editor, send, receive) = super::super::tests::fixture();
    let context = egui::Context::default();
    for _ in 0..3 {
        frame(&mut editor, &context, vec![]);
    }
    (editor, context, send, receive)
}

fn click(editor: &mut Editor, context: &egui::Context, label: &str) {
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
    let (mut editor, context, _send, receive) = initialized();
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
    assert_eq!(editor.document.project, before);
    assert!(receive.try_recv().is_err());
}

#[test]
fn a_clean_system_quit_approves_without_saving_again() {
    let (mut editor, context, _send, receive) = initialized();
    let directory = tempfile::tempdir().unwrap();
    editor.document = editor
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
    assert!(receive.try_recv().is_err());
}

#[test]
fn blocked_system_quit_never_accepts_a_tool_or_replaces_an_existing_decision() {
    for gate in 0..4 {
        let (mut editor, context, _send, receive) = initialized();
        match gate {
            0 => editor.busy = true,
            1 => editor.pick = Some(Pick::Dmin),
            2 => editor.transition = Some(Transition::New),
            3 => editor.report_failure(FailureKind::Save, "fixture failure".into()),
            _ => unreachable!(),
        }
        let before = editor.document.project.clone();
        host::App::request_quit(&mut editor);
        frame(&mut editor, &context, vec![]);
        assert_eq!(
            host::App::quit_state(&editor),
            Some(host::QuitState::Running)
        );
        assert!(!editor.allow_close && editor.menus.pending.is_empty());
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.pick, (gate == 1).then_some(Pick::Dmin));
        assert_eq!(editor.transition, (gate == 2).then_some(Transition::New));
        assert!(receive.try_recv().is_err());
    }
}

#[test]
fn system_quit_waits_for_save_success_and_cannot_approve_a_failed_or_stale_save() {
    for success in [false, true] {
        let (mut editor, context, send, receive) = initialized();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.antitone");
        editor.document = editor
            .document
            .save(&path, crate::project::Save::New)
            .unwrap();
        editor.roll.exposure = 0.25;
        editor.sync_document();
        let before = editor.document.project.clone();
        host::App::request_quit(&mut editor);
        frame(&mut editor, &context, vec![]);
        click(&mut editor, &context, "Save changes");
        assert_eq!(
            host::App::quit_state(&editor),
            Some(host::QuitState::Pending)
        );
        assert!(!editor.allow_close);
        let Command::Document(request, work) = receive.try_recv().unwrap() else {
            panic!("save was not dispatched")
        };
        let DocumentWork::Save(snapshot, destination, policy) = *work else {
            panic!("not a save")
        };
        send.send(Message::Saved(
            request + 1,
            Err("stale save failure".into()),
        ))
        .unwrap();
        editor.receive();
        assert_eq!(
            host::App::quit_state(&editor),
            Some(host::QuitState::Pending)
        );
        let result = if success {
            Ok(Box::new(snapshot.save(&destination, policy).unwrap()))
        } else {
            Err("fixture write failure".into())
        };
        send.send(Message::Saved(request, result)).unwrap();
        editor.receive();
        if success {
            assert_eq!(
                host::App::quit_state(&editor),
                Some(host::QuitState::Pending)
            );
            frame(&mut editor, &context, vec![]);
            assert_eq!(
                host::App::quit_state(&editor),
                Some(host::QuitState::Approved)
            );
            assert!(!editor.document.dirty());
        } else {
            assert_eq!(
                host::App::quit_state(&editor),
                Some(host::QuitState::Running)
            );
            assert!(!editor.allow_close && editor.document.dirty());
            assert_eq!(editor.document.project, before);
            assert!(editor.failure.is_some());
        }
        assert!(receive.try_recv().is_err());
    }
}

#[test]
fn cancelling_project_replacement_retires_the_native_close_continuation() {
    let (mut editor, context, _send, receive) = initialized();
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
    assert!(receive.try_recv().is_err());
}
