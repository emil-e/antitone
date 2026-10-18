use super::*;

fn frame(editor: &mut Editor, context: &egui::Context) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| host::App::ui(editor, ui),
    );
    output.textures_delta.clear();
}

#[test]
fn native_history_actions_use_dynamic_labels_and_advance_only_after_preparation() {
    let (mut editor, send, receiver) = super::super::tests::fixture();
    let context = egui::Context::default();
    frame(&mut editor, &context);
    frame(&mut editor, &context);
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.5;
    editor.record_edit(before.clone(), "Roll exposure", None);
    let state = editor.application_menu_state();
    assert_eq!(state.label(Action::Undo), "Undo Roll exposure");
    assert!(state.enabled(Action::Undo));
    assert!(!state.enabled(Action::Redo));
    host::App::native_menu_installed(&mut editor);
    host::App::menu_action(&mut editor, Action::Undo);
    frame(&mut editor, &context);
    let Command::Document(request, work) = receiver.try_recv().unwrap() else {
        panic!("history dispatch")
    };
    let DocumentWork::History(document, selected, history::Direction::Undo) = *work else {
        panic!("history candidate")
    };
    assert_eq!(document.project, before);
    assert_eq!(
        editor.roll.exposure, 0.5,
        "preparation must not replace accepted state"
    );
    assert!(!editor.application_menu_state().enabled(Action::Undo));
    send.send(Message::Prepared(
        request,
        Ok(Box::new(Prepared {
            document,
            selected,
            ready: None,
            reuse_ready: false,
            correction: None,
            preserve_view: true,
        })),
    ))
    .unwrap();
    frame(&mut editor, &context);
    assert_eq!(editor.roll.exposure, 0.);
    let state = editor.application_menu_state();
    assert_eq!(state.label(Action::Undo), "Undo");
    assert_eq!(state.label(Action::Redo), "Redo Roll exposure");
    assert!(!state.enabled(Action::Undo));
    assert!(state.enabled(Action::Redo));
    editor.pick = Some(Pick::Dmin);
    host::App::menu_action(&mut editor, Action::Redo);
    frame(&mut editor, &context);
    assert!(
        receiver.try_recv().is_err(),
        "a queued native command must not bypass a draft"
    );
    assert_eq!(editor.roll.exposure, 0.);
}

#[test]
fn queued_native_history_checks_current_pointer_input_before_the_menu_state_catches_up() {
    let (mut editor, _, receiver) = super::super::tests::fixture();
    let context = egui::Context::default();
    frame(&mut editor, &context);
    frame(&mut editor, &context);
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.5;
    editor.record_edit(before, "Roll exposure", None);
    host::App::native_menu_installed(&mut editor);
    assert!(editor.application_menu_state().enabled(Action::Undo));
    host::App::menu_action(&mut editor, Action::Undo);
    super::super::tests::app_frame(
        &mut editor,
        &context,
        vec![egui::Event::PointerButton {
            pos: egui::pos2(0., 0.),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(receiver.try_recv().is_err());
    assert_eq!(editor.roll.exposure, 0.5);
    assert!(!editor.application_menu_state().enabled(Action::Undo));
}

#[test]
fn native_undo_and_redo_target_text_without_surrendering_document_history() {
    let (mut editor, _, receiver) = super::super::tests::fixture();
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.5;
    editor.record_edit(before, "Roll exposure", None);
    let accepted = editor.document.project.clone();
    editor.menus.text_focused = true;
    let state = editor.application_menu_state();
    assert_eq!(state.label(Action::Undo), "Undo");
    assert_eq!(state.label(Action::Redo), "Redo");
    for action in [Action::Undo, Action::Redo] {
        assert!(state.enabled(action));
        host::App::menu_action(&mut editor, action);
        let events = host::App::menu_input(&mut editor, None);
        assert!(
            matches!(events.as_slice(), [egui::Event::Key { key: egui::Key::Z, pressed: true, modifiers, .. }]
            if modifiers.command && modifiers.shift == (action == Action::Redo))
        );
        assert_eq!(editor.document.project, accepted);
        assert!(receiver.try_recv().is_err());
    }
    editor.menus.text_focused = false;
    assert_eq!(
        editor.application_menu_state().label(Action::Undo),
        "Undo Roll exposure"
    );
}

#[test]
fn native_quit_and_close_wait_for_the_document_decision() {
    for action in [Action::Quit, Action::Close] {
        let (mut editor, _send, receiver) = super::super::tests::fixture();
        let context = egui::Context::default();
        frame(&mut editor, &context);
        host::App::native_menu_installed(&mut editor);
        host::App::menu_action(&mut editor, action);
        frame(&mut editor, &context);
        assert_eq!(editor.transition, Some(Transition::Close));
        assert!(!editor.allow_close);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn queued_native_actions_revalidate_tool_and_modal_availability() {
    let (mut editor, _send, receiver) = super::super::tests::fixture();
    let context = egui::Context::default();
    frame(&mut editor, &context);
    editor.pick = Some(Pick::Dmin);
    for action in [
        Action::New,
        Action::Open,
        Action::AddScans,
        Action::Save,
        Action::Quit,
    ] {
        assert!(!editor.application_menu_state().enabled(action));
        host::App::menu_action(&mut editor, action);
        frame(&mut editor, &context);
        assert!(receiver.try_recv().is_err());
        assert!(editor.transition.is_none());
    }
    assert_eq!(editor.pick, Some(Pick::Dmin));
    editor.pick = None;
    editor.transition = Some(Transition::New);
    host::App::menu_action(&mut editor, Action::Settings);
    frame(&mut editor, &context);
    assert!(editor.menus.panel.is_none());
    assert_eq!(editor.transition, Some(Transition::New));
}

#[test]
fn closing_settings_preserves_the_document_and_quit_still_uses_its_guard() {
    let (mut editor, _send, receiver) = super::super::tests::fixture();
    let context = egui::Context::default();
    frame(&mut editor, &context);
    frame(&mut editor, &context);
    let before = editor.document.project.clone();
    for action in [Action::Settings, Action::Close] {
        host::App::menu_action(&mut editor, action);
        frame(&mut editor, &context);
    }
    assert!(editor.menus.panel.is_none());
    assert!(!editor.allow_close);
    assert!(editor.transition.is_none());
    assert_eq!(editor.document.project, before);
    host::App::menu_action(&mut editor, Action::Settings);
    frame(&mut editor, &context);
    host::App::menu_action(&mut editor, Action::Quit);
    frame(&mut editor, &context);
    frame(&mut editor, &context);
    assert!(editor.menus.panel.is_none());
    assert_eq!(editor.transition, Some(Transition::Close));
    assert!(!editor.allow_close);
    assert!(receiver.try_recv().is_err());
}

#[test]
fn scopes_menu_updates_the_same_preference_without_dirtying_the_project() {
    let (mut editor, _send, _receiver) = super::super::tests::fixture();
    let context = egui::Context::default();
    frame(&mut editor, &context);
    frame(&mut editor, &context);
    let before = editor.document.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    editor.preference_store = Some(preferences::Store::open(path.clone()).unwrap().0);
    let visible = editor.scopes.visible;
    host::App::menu_action(&mut editor, Action::Scopes);
    frame(&mut editor, &context);
    assert_eq!(editor.scopes.visible, !visible);
    assert_eq!(
        editor.application_menu_state().checked(Action::Scopes),
        Some(!visible)
    );
    assert_eq!(
        preferences::Store::open(path).unwrap().1.scopes_visible,
        !visible
    );
    assert_eq!(editor.document.project, before);
}
