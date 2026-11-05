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
    let viewport = editor.document.visit.canvas.viewport;
    let preferences = editor.preferences.clone();
    let project = editor.document.state.project.clone();
    let before = (
        editor.document.source,
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
    editor.document.visit.canvas.viewport.fit();
    editor.document.source = !editor.document.source;
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
    assert_eq!(editor.document.visit.canvas.viewport, viewport);
    assert_eq!(
        (
            editor.document.source,
            editor.source_exposure,
            editor.source_white_balance,
            editor.tab
        ),
        before
    );
    assert_eq!(editor.preferences, preferences);
    assert_eq!(editor.document.state.project, project);
}

#[test]
fn requested_exit_waits_for_host_acknowledgement_and_accepts_os_adjusted_bounds() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    let viewport = editor.document.visit.canvas.viewport;
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
    assert_eq!(editor.document.visit.canvas.viewport, viewport);
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
            editor.preview_picker(PickerCase::Dmin);
        }
        frame(&mut editor, &context, Some(false), vec![]);
        let viewport = editor.document.visit.canvas.viewport;
        let project = editor.document.state.project.clone();
        let draft = editor.document.crop_edit().map(|edit| edit.draft);
        let pick = editor.test_pick();
        frame(&mut editor, &context, Some(false), key(egui::Key::F));
        frame(&mut editor, &context, Some(true), vec![]);
        frame(&mut editor, &context, Some(true), key(egui::Key::Enter));
        assert_eq!(editor.document.crop_edit().map(|edit| edit.draft), draft);
        assert_eq!(editor.test_pick(), pick);
        frame(&mut editor, &context, Some(true), key(egui::Key::Escape));
        editor.fullscreen_changed(false);
        frame(&mut editor, &context, Some(false), vec![]);
        assert!(editor.fullscreen.is_none());
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        assert_eq!(editor.document.crop_edit().map(|edit| edit.draft), draft);
        assert_eq!(editor.test_pick(), pick);
        assert_eq!(editor.document.state.project, project);
    }
}

#[test]
fn fullscreen_preserves_normal_navigation_anchor_with_or_without_a_hosted_interaction() {
    for dust in [false, true] {
        let mut editor = ready();
        editor.tab = 1;
        let context = egui::Context::default();
        if dust {
            editor.begin_dust_cleanup();
            assert!(editor.dusting());
        }
        frame(&mut editor, &context, Some(false), vec![]);
        editor.document.visit.canvas.viewport.zoom = Some(2.);
        frame(&mut editor, &context, Some(false), vec![]);
        let layout = editor.active_canvas().last_layout.unwrap().0;
        editor
            .document
            .visit
            .canvas
            .viewport
            .pan(layout, 1., egui::vec2(80., -20.));
        frame(&mut editor, &context, Some(false), vec![]);
        let viewport = editor.document.visit.canvas.viewport;
        let before = editor.active_canvas().last_layout.unwrap().0;
        let anchor = before
            .composition
            .frame_to_source(before.frame_point(before.canvas.center()));
        frame(&mut editor, &context, Some(false), key(egui::Key::F));
        frame_with_size(
            &mut editor,
            &context,
            Some(true),
            egui::vec2(1920., 1080.),
            vec![],
        );
        let after = editor.active_canvas().last_layout.unwrap().0;
        let actual = after
            .composition
            .frame_to_source(after.frame_point(after.canvas.center()));
        assert_eq!(after.scale, before.scale);
        for axis in 0..2 {
            assert!(
                (actual.0[axis] - anchor.0[axis]).abs() < 1e-3,
                "fullscreen moved the source anchor: {anchor:?} -> {actual:?}, Dust={dust}"
            );
        }
        frame(&mut editor, &context, Some(true), key(egui::Key::Escape));
        editor.fullscreen_changed(false);
        frame(&mut editor, &context, Some(false), vec![]);
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        assert_eq!(editor.dusting(), dust);
    }
}

#[test]
fn fullscreen_navigation_uses_normal_view_and_preserves_the_suspended_crop_view() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    editor.document.visit.canvas.viewport.zoom = Some(2.);
    frame(&mut editor, &context, Some(false), vec![]);
    let normal = editor.document.visit.canvas.viewport;
    editor.begin_crop();
    editor
        .document
        .crop_edit_mut()
        .unwrap()
        .install_proposal(Crop::from_region(Rect {
            x: 100,
            y: 90,
            width: 400,
            height: 300,
        }));
    editor.document.visit.canvas.last_layout = None;
    editor.invalidate_original();
    frame(&mut editor, &context, Some(false), vec![]);
    let draft = editor.document.crop_edit().unwrap().draft;
    let crop_view = editor.with_tool_viewport(|viewport| *viewport);
    assert_ne!(crop_view, normal);
    let project = editor.document.state.project.clone();
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    frame(&mut editor, &context, Some(true), vec![]);
    let composition = editor.active_canvas().last_layout.unwrap().0.composition;
    assert_eq!(composition.crop(), Crop::from_region(composition.area()));
    let layout = editor.active_canvas().last_layout.unwrap().0;
    let center = layout.canvas.center();
    frame(
        &mut editor,
        &context,
        Some(true),
        vec![egui::Event::PointerMoved(center), egui::Event::Zoom(1.25)],
    );
    let zoomed = editor.document.visit.canvas.viewport;
    assert!(zoomed.zoom.unwrap() > normal.zoom.unwrap());
    frame(&mut editor, &context, Some(true), vec![]);
    assert_eq!(
        editor.document.visit.canvas.viewport, zoomed,
        "next layout must not discard fullscreen zoom"
    );
    for (point, pressed) in [
        (center, true),
        (center + egui::vec2(40., 25.), true),
        (center + egui::vec2(40., 25.), false),
    ] {
        let mut events = vec![egui::Event::PointerMoved(point)];
        if point == center || !pressed {
            events.push(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Middle,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        frame(&mut editor, &context, Some(true), events);
    }
    let panned = editor.document.visit.canvas.viewport;
    assert_ne!(panned, zoomed);
    frame(&mut editor, &context, Some(true), vec![]);
    assert_eq!(editor.document.visit.canvas.viewport, panned);
    let output = frame(&mut editor, &context, Some(true), vec![]);
    let fit = super::super::tests::ui_labels(output)
        .into_iter()
        .find(|(name, _)| name == "Fit")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        frame(
            &mut editor,
            &context,
            Some(true),
            vec![
                egui::Event::PointerMoved(fit),
                egui::Event::PointerButton {
                    pos: fit,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(editor.document.visit.canvas.viewport.zoom.is_none());
    frame(
        &mut editor,
        &context,
        Some(true),
        vec![egui::Event::PointerMoved(center), egui::Event::Zoom(2.)],
    );
    assert!(editor.document.visit.canvas.viewport.zoom.is_some());
    frame(&mut editor, &context, Some(true), key(egui::Key::Z));
    assert!(editor.document.visit.canvas.viewport.zoom.is_none());
    frame(&mut editor, &context, Some(true), key(egui::Key::Escape));
    editor.fullscreen_changed(false);
    frame(&mut editor, &context, Some(false), vec![]);
    assert!(editor.fullscreen.is_none());
    assert_eq!(editor.document.visit.canvas.viewport, normal);
    assert_eq!(editor.with_tool_viewport(|viewport| *viewport), crop_view);
    assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
    assert_eq!(editor.document.state.project, project);
}

#[test]
fn preexisting_native_fullscreen_is_not_changed_by_photo_only_viewing() {
    for native_before in [false, true] {
        let mut editor = ready();
        let context = egui::Context::default();
        frame(&mut editor, &context, Some(native_before), vec![]);
        let viewport = editor.document.visit.canvas.viewport;
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
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
    }
}

#[test]
fn ending_reference_fullscreen_does_not_restore_its_view_over_the_working_photo() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    frame(&mut editor, &context, Some(false), vec![]);
    let working = editor.document.visit.canvas.viewport;
    let reference = editor.document.visit.canvas.ready.as_ref().unwrap().clone();
    super::super::tests::paused_reference_fixture(&mut editor, reference, None);
    editor.with_tool_viewport(|viewport| viewport.zoom = Some(3.));
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    assert!(editor.fullscreen.is_some());
    editor.finish_flat_edit();
    frame(&mut editor, &context, Some(false), vec![]);
    assert!(editor.fullscreen.is_none());
    assert_eq!(editor.document.visit.canvas.viewport, working);
}

#[test]
fn a_changed_source_retires_fullscreen_without_restoring_the_old_source_view() {
    let mut editor = ready();
    let context = egui::Context::default();
    frame(&mut editor, &context, Some(false), vec![]);
    frame(&mut editor, &context, Some(false), key(egui::Key::F));
    editor.id += 1;
    editor.document.visit.canvas.viewport.zoom = Some(3.);
    editor.source_exposure = 5.;
    frame(&mut editor, &context, Some(false), vec![]);
    assert!(editor.fullscreen.is_none());
    assert!(!editor.fullscreen_requested());
    assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(3.));
    assert_eq!(editor.source_exposure, 5.);
}

#[test]
fn completed_new_cannot_reuse_the_departed_photos_fullscreen_evidence_before_a_paint() {
    let editor = super::super::tests::authored_fixture(&["patches.dng"]);
    crate::application::test_support::with_application(editor, |app| {
        let context = egui::Context::default();
        frame(app, &context, Some(false), vec![]);
        app.read(|editor| {
            assert!(editor.canvas_frame.layout.is_some());
            assert!(editor.fullscreen_available());
        });
        app.enter(|editor, cx| editor.start_document(Preparation::New, cx));
        app.run_until(Duration::from_secs(20), |editor| {
            editor.document_pending.is_none()
        });
        app.update(|editor| {
            assert!(!editor.interaction_blocked() && editor.document.project.order.is_empty());
            assert!(editor.selected_view().is_none());
            assert!(!editor.fullscreen_available());
            editor.toggle_fullscreen(&context);
            assert!(editor.fullscreen.is_none());
        });
    });
}

#[test]
fn cold_photo_switch_cannot_reuse_the_previous_photos_fullscreen_evidence_before_a_paint() {
    let editor = super::super::tests::authored_fixture(&["patches.dng", "roll-0.dng"]);
    crate::application::test_support::with_application(editor, |app| {
        let hold = crate::background::testing::pause();
        let context = egui::Context::default();
        frame(app, &context, Some(false), vec![]);
        let before = app.read(|editor| {
            assert!(editor.canvas_frame.layout.is_some());
            assert!(editor.fullscreen_available());
            editor.document.visit.current_source
        });
        app.enter(|editor, cx| editor.select_roll_photo(1, egui::Modifiers::NONE, cx));
        app.update(|editor| {
            assert_ne!(editor.document.visit.current_source, before);
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(
                editor.selected_view().is_none(),
                "fixture must exercise a cold switch without a usable placeholder"
            );
            assert!(!editor.fullscreen_available());
            editor.toggle_fullscreen(&context);
            assert!(editor.fullscreen.is_none());
        });
        drop(hold);
        app.run_until(Duration::from_secs(20), |editor| {
            editor.document.visit.canvas.ready.is_some()
        });
        app.read(|editor| {
            assert!(
                editor.fullscreen_available(),
                "the prepared successor may be viewed fullscreen"
            )
        });
    });
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
