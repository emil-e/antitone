use super::*;

fn frame(
    context: &egui::Context,
    time: f64,
    enabled: bool,
    events: Vec<egui::Event>,
) -> Option<egui::Response> {
    frame_height(context, time, enabled, events, 0.)
}

fn frame_height(
    context: &egui::Context,
    time: f64,
    enabled: bool,
    events: Vec<egui::Event>,
    height: f32,
) -> Option<egui::Response> {
    super::super::configure_fonts(context);
    let mut response = None;
    let mut output = context.run_ui(
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            if !enabled {
                ui.disable();
            }
            response = ViewerBar::Overlay.show(ui, |ui| {
                ui.set_min_height(height);
                ui.button("Viewer action")
            });
        },
    );
    output.textures_delta.clear();
    response
}

fn hide(context: &egui::Context, time: f64) {
    for dt in [0., 0.1, 0.3, 0.5] {
        frame(context, time + dt, true, vec![]);
    }
}

#[test]
fn overlay_hides_without_leaving_a_canvas_hitbox_and_resets_on_entry() {
    let context = egui::Context::default();
    frame(&context, 0., true, vec![]);
    let response = frame(&context, 0.1, true, vec![]).unwrap();
    let point = response.rect.center();
    hide(&context, 3.);
    assert!(frame(&context, 4., true, vec![]).is_none());
    assert_ne!(
        context.layer_id_at(point),
        Some(egui::LayerId::new(egui::Order::Foreground, overlay_id()))
    );
    ViewerBar::reset_overlay(&context);
    frame(&context, 5., true, vec![]);
    assert!(frame(&context, 5.3, true, vec![]).is_some());
}

#[test]
fn hover_retention_covers_a_tall_activity_overlay() {
    let context = egui::Context::default();
    frame_height(&context, 0., true, vec![], 100.);
    let response = frame_height(&context, 0.1, true, vec![], 100.).unwrap();
    let point = response.rect.center();
    assert!(point.y < 600. - VIEWER_BAR_HEIGHT - 24.);
    frame_height(
        &context,
        1.,
        true,
        vec![egui::Event::PointerMoved(point)],
        100.,
    );
    assert!(frame_height(&context, 5., true, vec![], 100.).is_some());
}

#[test]
fn bottom_hover_and_keyboard_focus_keep_the_overlay_visible_without_enabling_disabled_input() {
    let context = egui::Context::default();
    frame(&context, 0., true, vec![]);
    hide(&context, 3.);
    frame(
        &context,
        4.,
        false,
        vec![egui::Event::PointerMoved(egui::pos2(400., 570.))],
    );
    let response = frame(&context, 4.3, false, vec![]).unwrap();
    assert!(!response.enabled());
    context.memory_mut(|memory| memory.request_focus(response.id));
    frame(
        &context,
        5.,
        true,
        vec![egui::Event::PointerMoved(egui::pos2(400., 200.))],
    );
    assert!(frame(&context, 8., true, vec![]).is_some());
    context.memory_mut(|memory| memory.surrender_focus(response.id));
    hide(&context, 11.);
    assert!(frame(&context, 12., true, vec![]).is_none());
}
