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
