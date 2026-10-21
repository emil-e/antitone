use super::*;
const AREA: Rect = Rect {
    x: 12,
    y: 12,
    width: 8640,
    height: 5760,
};

struct Harness {
    editor: Editor,
    context: egui::Context,
    time: f64,
    enabled: bool,
}

impl Harness {
    fn new() -> Self {
        let editor = Editor::state();
        Self {
            editor,
            context: egui::Context::default(),
            time: 0.,
            enabled: true,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (CanvasLayout, bool, egui::CursorIcon) {
        self.time += 1. / 60.;
        let mut result = None;
        let mut output = self.context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 600.),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        if !self.enabled {
                            ui.disable();
                        }
                        let (canvas, response) = ui.allocate_exact_size(
                            ui.available_size(),
                            egui::Sense::click_and_drag(),
                        );
                        let geometry = self.editor.viewport.layout(canvas, AREA, 2.);
                        result = Some(self.editor.navigate(ui, &response, geometry, 2.));
                    });
            },
        );
        let cursor = output.platform_output.cursor_icon;
        output.textures_delta.clear();
        let (geometry, panning) = result.unwrap();
        (geometry, panning, cursor)
    }

    fn press(
        &mut self,
        point: egui::Pos2,
        pressed: bool,
        button: egui::PointerButton,
    ) -> (CanvasLayout, bool, egui::CursorIcon) {
        self.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ])
    }

    fn native(&mut self) -> CanvasLayout {
        let (fit, _, _) = self.frame(vec![]);
        self.editor
            .viewport
            .zoom_at(fit, 2., 1., fit.canvas.center());
        self.frame(vec![]).0
    }
}

fn wheel(delta: egui::Vec2, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        modifiers,
        phase: egui::TouchPhase::Move,
    }
}

#[test]
fn plain_scroll_pans_both_axes_without_changing_zoom_or_fit_mode() {
    let mut h = Harness::new();
    let initial = h.native();
    let anchor = initial.canvas.center();
    let (after, _, _) = h.frame(vec![
        egui::Event::PointerMoved(anchor),
        wheel(egui::vec2(30., -20.), egui::Modifiers::NONE),
    ]);
    assert_eq!(after.scale, initial.scale);
    let delta = h.context.input(|input| input.translation_delta());
    assert!(delta.x > 0. && delta.y < 0.);
    assert!((after.image.min - initial.image.min - delta).length() < 0.001);
    h.editor.viewport.fit();
    h.frame(vec![wheel(egui::vec2(30., -20.), egui::Modifiers::NONE)]);
    assert!(h.editor.viewport.zoom.is_none());
}

#[test]
fn modified_wheel_and_native_pinch_use_egui_scale_once_and_preserve_anchor() {
    for modifiers in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
        let mut h = Harness::new();
        let initial = h.native();
        let point = egui::pos2(420., 260.);
        let before = initial.source_point(point);
        let (after, _, _) = h.frame(vec![
            egui::Event::PointerMoved(point),
            wheel(egui::vec2(0., 20.), modifiers),
        ]);
        let expected = h.context.input(|input| input.zoom_delta());
        assert!((after.scale / initial.scale - expected).abs() < 1e-6);
        assert!(expected > 1.);
        assert!((after.source_point(point) - before).length() < 0.001);
    }
    let mut h = Harness::new();
    let initial = h.native();
    let point = egui::pos2(420., 260.);
    let (after, _, _) = h.frame(vec![
        egui::Event::PointerMoved(point),
        egui::Event::Zoom(1.25),
    ]);
    assert!((after.scale / initial.scale - 1.25).abs() < 1e-6);
    assert!((after.source_point(point) - initial.source_point(point)).length() < 0.001);
}

fn touch(id: u64, phase: egui::TouchPhase, point: egui::Pos2) -> egui::Event {
    egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos: point,
        force: None,
    }
}

#[test]
fn multi_touch_preserves_previous_center_at_translated_center_and_ignores_synthetic_duplicates() {
    let mut h = Harness::new();
    let initial = h.native();
    let center = egui::pos2(430., 280.);
    h.frame(vec![egui::Event::PointerMoved(center)]);
    h.frame(vec![
        egui::Event::PointerMoved(center),
        touch(1, egui::TouchPhase::Start, center - egui::vec2(50., 0.)),
        touch(2, egui::TouchPhase::Start, center + egui::vec2(50., 0.)),
    ]);
    let moved = center + egui::vec2(15., 10.);
    let (after, _, _) = h.frame(vec![
        touch(1, egui::TouchPhase::Move, moved - egui::vec2(75., 0.)),
        touch(2, egui::TouchPhase::Move, moved + egui::vec2(75., 0.)),
        egui::Event::Zoom(1.5),
        wheel(egui::vec2(15., 10.), egui::Modifiers::NONE),
    ]);
    assert!(
        (after.scale / initial.scale - 1.5).abs() < 1e-6,
        "scale={} touch={:?}",
        after.scale / initial.scale,
        h.context.input(|input| input.multi_touch())
    );
    assert!((after.source_point(moved) - initial.source_point(center)).length() < 0.001);
}

#[test]
fn click_toggles_native_and_fit_and_primary_drag_pans_only_without_a_picker() {
    let mut h = Harness::new();
    let (fit, _, _) = h.frame(vec![]);
    let point = egui::pos2(430., 280.);
    assert_eq!(
        h.frame(vec![egui::Event::PointerMoved(point)]).2,
        egui::CursorIcon::ZoomIn
    );
    h.press(point, true, egui::PointerButton::Primary);
    let (native, _, cursor) = h.press(point, false, egui::PointerButton::Primary);
    assert_eq!(h.editor.viewport.zoom, Some(1.));
    assert_eq!(cursor, egui::CursorIcon::Grab);
    assert!((native.source_point(point) - fit.source_point(point)).length() < 0.001);
    h.press(point, true, egui::PointerButton::Primary);
    let delta = egui::vec2(20., 15.);
    let (dragged, panning, cursor) = h.frame(vec![egui::Event::PointerMoved(point + delta)]);
    assert!(panning);
    assert_eq!(cursor, egui::CursorIcon::Grabbing);
    assert!((dragged.image.min - native.image.min - delta).length() < 0.001);
    h.press(point + delta, false, egui::PointerButton::Primary);
    h.press(point, true, egui::PointerButton::Primary);
    h.press(point, false, egui::PointerButton::Primary);
    assert!(h.editor.viewport.zoom.is_none());
    h.native();
    h.editor.pick = Some(Pick::Dmin);
    h.press(point, true, egui::PointerButton::Primary);
    let (before, _, _) = h.frame(vec![]);
    let (after, panning, cursor) = h.frame(vec![egui::Event::PointerMoved(point + delta)]);
    assert!(!panning);
    assert_eq!(cursor, egui::CursorIcon::Crosshair);
    assert_eq!(after.image, before.image);
}

#[test]
fn pan_hover_preserves_fit_small_zoom_picker_and_disabled_cursors() {
    let mut h = Harness::new();
    let (fit, _, _) = h.frame(vec![]);
    let point = fit.canvas.center();
    assert_eq!(
        h.frame(vec![egui::Event::PointerMoved(point)]).2,
        egui::CursorIcon::ZoomIn
    );
    h.editor.viewport.zoom_at(fit, 2., 0.1, point);
    let (small, _, cursor) = h.frame(vec![egui::Event::PointerMoved(point)]);
    assert!(!can_pan(small));
    assert_eq!(cursor, egui::CursorIcon::ZoomOut);
    h.native();
    assert_eq!(
        h.frame(vec![egui::Event::PointerMoved(point)]).2,
        egui::CursorIcon::Grab
    );
    h.editor.pick = Some(Pick::Gamma);
    assert_eq!(
        h.frame(vec![egui::Event::PointerMoved(point)]).2,
        egui::CursorIcon::Crosshair
    );
    h.editor.busy = true;
    assert_eq!(
        h.frame(vec![egui::Event::PointerMoved(point)]).2,
        egui::CursorIcon::Default
    );
    h.editor.pick = None;
    h.editor.busy = false;
    h.enabled = false;
    assert_eq!(
        h.frame(vec![egui::Event::PointerMoved(point)]).2,
        egui::CursorIcon::Default
    );
}

#[test]
fn picker_cursors_share_rectangle_hit_testing_and_no_image_click_changes_view() {
    let mut h = Harness::new();
    let geometry = h.native();
    let region = Rect {
        x: 4000,
        y: 2600,
        width: 256,
        height: 256,
    };
    let screen = geometry.screen_region(region);
    let region = Coverage::rectangle(region);
    assert_eq!(
        picker_cursor(geometry, screen.min, Some(region), None, false),
        egui::CursorIcon::ResizeNwSe
    );
    assert_eq!(
        picker_cursor(geometry, screen.right_top(), Some(region), None, false),
        egui::CursorIcon::ResizeNeSw
    );
    assert_eq!(
        picker_cursor(geometry, screen.center(), Some(region), None, false),
        egui::CursorIcon::Grab
    );
    assert_eq!(
        picker_cursor(
            geometry,
            screen.center(),
            Some(region),
            Some(PickDrag::begin(geometry, screen.center(), Some(region))),
            true
        ),
        egui::CursorIcon::Grabbing
    );
    h.editor.pick = Some(Pick::Dmax);
    let zoom = h.editor.viewport.zoom;
    h.press(geometry.canvas.center(), true, egui::PointerButton::Primary);
    h.press(
        geometry.canvas.center(),
        false,
        egui::PointerButton::Primary,
    );
    assert_eq!(h.editor.viewport.zoom, zoom);
}

#[test]
fn middle_and_space_primary_drag_pan_during_picking() {
    for button in [egui::PointerButton::Middle, egui::PointerButton::Primary] {
        let mut h = Harness::new();
        let initial = h.native();
        h.editor.pick = Some(Pick::Gamma);
        let point = initial.canvas.center();
        h.press(point, true, button);
        let mut events = vec![egui::Event::PointerMoved(point + egui::vec2(20., 10.))];
        if button == egui::PointerButton::Primary {
            events.push(egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let (after, panning, _) = h.frame(events);
        assert!(panning);
        assert!((after.image.min - initial.image.min - egui::vec2(20., 10.)).length() < 0.001);
    }
}

#[test]
fn z_fits_without_text_focus_but_does_not_intercept_numeric_text() {
    let mut h = Harness::new();
    super::super::tests::app_frame(&mut h.editor, &h.context, vec![]);
    h.native();
    let key = || egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut output = h.context.run_ui(
        egui::RawInput {
            events: vec![key()],
            ..Default::default()
        },
        |ui| crate::editor::tests::paint_controller(&mut h.editor, ui),
    );
    output.textures_delta.clear();
    assert!(h.editor.viewport.zoom.is_none());
    h.native();
    let focus = egui::Id::new("numeric");
    h.context.memory_mut(|memory| memory.request_focus(focus));
    let mut text = String::new();
    let mut output = h.context.run_ui(
        egui::RawInput {
            events: vec![key(), egui::Event::Text("z".into())],
            ..Default::default()
        },
        |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(focus));
            crate::editor::tests::paint_controller(&mut h.editor, ui);
        },
    );
    output.textures_delta.clear();
    assert_eq!(h.editor.viewport.zoom, Some(1.));
    assert_eq!(text, "z");
}
