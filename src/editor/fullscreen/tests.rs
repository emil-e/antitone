use super::*;

fn frame(
    editor: &mut impl crate::editor::tests::UiFixture,
    context: &egui::Context,
    native: Option<bool>,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    frame_with_size(editor, context, native, egui::vec2(1100., 800.), events)
}

fn frame_with_size(
    editor: &mut impl crate::editor::tests::UiFixture,
    context: &egui::Context,
    native: Option<bool>,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    context.enable_accesskit();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut input = egui::RawInput {
        screen_rect: Some(rect),
        events,
        ..Default::default()
    };
    let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
    viewport.fullscreen = native;
    viewport.inner_rect = Some(rect);
    let mut output = context.run_ui(input, |ui| editor.ui(ui));
    output.textures_delta.clear();
    output
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

fn ready() -> Editor {
    let mut editor = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    editor
}

#[test]
fn fullscreen_hides_workspace_and_restores_view_after_window_return() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    frame(&mut editor, &context, Some(false), vec![]);
    let viewport = editor.viewport;
    let preferences = editor.preferences.clone();
    let project = editor.document.project.clone();
    let before = (
        editor.source,
        editor.source_exposure,
        editor.source_white_balance,
        editor.tab,
    );
    let output = frame(&mut editor, &context, Some(false), key(egui::Key::F));
    assert!(editor.fullscreen_requested());
    assert!(
        !output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Fullscreen(_)))
    );
    assert!(editor.fullscreen.is_some());
    let output = frame(&mut editor, &context, Some(false), vec![]);
    assert!(
        editor.fullscreen.is_some(),
        "a windowed native flag does not end photo-only viewing"
    );
    let labels = super::super::tests::ui_labels(output);
    assert!(!labels.iter().any(|(label, _)| matches!(
        label.as_str(),
        "Export" | "Roll" | "Photo" | "Rendering" | "Scopes"
    )));
    assert!(labels.iter().any(|(label, _)| label == "Exit fullscreen"));
    let fullscreen_size = egui::vec2(1920., 1080.);
    frame_with_size(&mut editor, &context, Some(false), fullscreen_size, vec![]);
    editor.viewport.fit();
    editor.source = !editor.source;
    editor.source_exposure = 2.;
    editor.source_white_balance = crate::raw::SourceWhiteBalance::AsShot;
    frame_with_size(
        &mut editor,
        &context,
        Some(false),
        fullscreen_size,
        key(egui::Key::Escape),
    );
    assert!(!editor.fullscreen_requested());
    assert!(
        editor.fullscreen.is_some(),
        "retain the fullscreen view until window restoration"
    );
    editor.fullscreen_changed(false);
    frame(&mut editor, &context, Some(false), vec![]);
    assert!(editor.fullscreen.is_none());
    assert_eq!(editor.viewport, viewport);
    assert_eq!(
        (
            editor.source,
            editor.source_exposure,
            editor.source_white_balance,
            editor.tab
        ),
        before
    );
    assert_eq!(editor.preferences, preferences);
    assert_eq!(editor.document.project, project);
}

#[test]
fn requested_exit_waits_for_host_acknowledgement_and_accepts_os_adjusted_bounds() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    let viewport = editor.viewport;
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    let fullscreen_size = egui::vec2(1920., 1080.);
    frame_with_size(&mut editor, &context, Some(true), fullscreen_size, vec![]);
    editor.source_exposure = 3.;
    frame_with_size(
        &mut editor,
        &context,
        Some(false),
        fullscreen_size,
        key(egui::Key::Escape),
    );
    frame_with_size(&mut editor, &context, Some(false), fullscreen_size, vec![]);
    assert!(
        editor.fullscreen.is_some(),
        "a windowed native flag is not geometry restoration"
    );
    assert_eq!(editor.source_exposure, 3.);
    frame(&mut editor, &context, Some(false), vec![]);
    assert!(
        editor.fullscreen.is_some(),
        "geometry alone cannot acknowledge restoration"
    );
    editor.fullscreen_changed(false);
    frame_with_size(
        &mut editor,
        &context,
        Some(false),
        egui::vec2(1000., 700.),
        vec![],
    );
    assert!(editor.fullscreen.is_none());
    assert_eq!(editor.viewport, viewport);
    assert_eq!(editor.source_exposure, 0.);
}

#[test]
fn fullscreen_suspends_crop_and_picker_without_escape_cancelling_them() {
    for crop in [false, true] {
        let mut editor = ready();
        let context = egui::Context::default();
        frame(&mut editor, &context, Some(false), vec![]);
        if crop {
            editor.begin_crop();
        } else {
            editor.begin_pick(Pick::Dmin, &crate::application::UiContext::render_only());
        }
        frame(&mut editor, &context, Some(false), vec![]);
        let viewport = editor.viewport;
        let project = editor.document.project.clone();
        let draft = editor
            .document
            .visit
            .crop_edit
            .as_ref()
            .map(|edit| edit.draft);
        let pick = editor.pick;
        frame(&mut editor, &context, Some(false), key(egui::Key::F));
        frame(&mut editor, &context, Some(true), vec![]);
        frame(&mut editor, &context, Some(true), key(egui::Key::Enter));
        assert_eq!(
            editor
                .document
                .visit
                .crop_edit
                .as_ref()
                .map(|edit| edit.draft),
            draft
        );
        assert_eq!(editor.pick, pick);
        frame(&mut editor, &context, Some(true), key(egui::Key::Escape));
        editor.fullscreen_changed(false);
        frame(&mut editor, &context, Some(false), vec![]);
        assert!(editor.fullscreen.is_none());
        assert_eq!(editor.viewport, viewport);
        assert_eq!(
            editor
                .document
                .visit
                .crop_edit
                .as_ref()
                .map(|edit| edit.draft),
            draft
        );
        assert_eq!(editor.pick, pick);
        assert_eq!(editor.document.project, project);
    }
}

#[test]
fn preexisting_native_fullscreen_is_not_changed_by_photo_only_viewing() {
    for native_before in [false, true] {
        let mut editor = ready();
        let context = egui::Context::default();
        frame(&mut editor, &context, Some(native_before), vec![]);
        let viewport = editor.viewport;
        frame(
            &mut editor,
            &context,
            Some(native_before),
            key(egui::Key::F),
        );
        frame(&mut editor, &context, Some(true), vec![]);
        let output = frame(
            &mut editor,
            &context,
            Some(native_before),
            key(egui::Key::F),
        );
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Fullscreen(_)))
        );
        editor.fullscreen_changed(false);
        frame(&mut editor, &context, Some(native_before), vec![]);
        assert!(editor.fullscreen.is_none());
        assert_eq!(editor.viewport, viewport);
    }
}

#[test]
fn a_changed_source_retires_fullscreen_without_restoring_the_old_source_view() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    editor.id += 1;
    editor.viewport.zoom = Some(3.);
    editor.source_exposure = 5.;
    frame(&mut editor, &context, Some(false), vec![]);
    assert!(editor.fullscreen.is_none());
    assert!(!editor.fullscreen_requested());
    assert_eq!(editor.viewport.zoom, Some(3.));
    assert_eq!(editor.source_exposure, 5.);
}

#[test]
fn unavailable_images_and_blocking_transactions_cannot_enter_fullscreen() {
    let mut editor = super::super::tests::fixture();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    assert!(editor.fullscreen.is_none());
    super::super::tests::add_ready_fixture(&mut editor);
    editor.transition = Some(Transition::New);
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    assert!(editor.fullscreen.is_none());
}
