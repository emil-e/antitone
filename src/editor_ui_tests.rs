#[path = "editor_ui_row_tests.rs"]
mod rows;
#[path = "editor_tool_group_tests.rs"]
mod tool_groups;

use super::TextAction;
use egui::{Context, Event, FullOutput, Modifiers, Pos2, Rect, Response, Ui};

fn frame<R>(
    context: &Context,
    width: f32,
    enabled: bool,
    events: Vec<Event>,
    mut draw: impl FnMut(&mut Ui) -> R,
) -> (R, FullOutput) {
    context.enable_accesskit();
    let mut result = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 200.))),
            events,
            ..Default::default()
        },
        |ui| {
            if !enabled {
                ui.disable();
            }
            result = Some(ui.horizontal(|ui| draw(ui)).inner);
        },
    );
    output.textures_delta.clear();
    (result.unwrap(), output)
}

fn direct(ui: &mut Ui, action: TextAction<'_>) -> Response {
    let mut button = egui::Button::new(action.label);
    if let Some(width) = action.toolbar_width {
        button = button.min_size(egui::vec2(width, 28.));
    }
    if action.primary {
        button = button.fill(ui.visuals().selection.bg_fill);
    }
    let response = ui.add_enabled(action.enabled, button);
    if let Some(name) = action.name {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), name)
        });
    }
    if let Some(help) = action.help {
        response.on_hover_text(help)
    } else {
        response
    }
}

fn key(key: egui::Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn pointer(position: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn text_actions_preserve_direct_button_geometry_ids_and_paint_at_both_scales() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for inherited_enabled in [true, false] {
                for action in [
                    TextAction::new("Use adjustable response").help("Explicit opt-in"),
                    TextAction::new("Choose reference RAW…"),
                    TextAction::new("Edit area…"),
                    TextAction::new("Clear"),
                    TextAction::new("Cancel").toolbar(0.),
                    TextAction::new("Apply").primary().toolbar(0.),
                    TextAction::new("Apply")
                        .primary()
                        .toolbar(0.)
                        .enabled(false),
                    TextAction::new("—").toolbar(50.).name("100%"),
                    TextAction::new("12345%").toolbar(50.).name("100%"),
                ] {
                    let context = Context::default();
                    context.set_pixels_per_point(scale);
                    frame(&context, width, inherited_enabled, vec![], |_| ());
                    let (baseline, before) =
                        frame(&context, width, inherited_enabled, vec![], |ui| {
                            direct(ui, action)
                        });
                    let (shared, after) = frame(&context, width, inherited_enabled, vec![], |ui| {
                        action.show(ui)
                    });
                    assert_eq!(shared.id, baseline.id);
                    assert_eq!(shared.rect, baseline.rect);
                    assert_eq!(shared.enabled(), baseline.enabled());
                    assert_eq!(after.shapes, before.shapes);
                    if let Some(min_width) = action.toolbar_width {
                        assert_eq!(shared.rect.height(), 28.);
                        assert!(shared.rect.width() >= min_width);
                    }
                }
            }
        }
    }
}

#[test]
fn toolbar_measurement_matches_allocation_without_advancing_layout_or_ids() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for custom_style in [false, true] {
                for framed in [true, false] {
                    let context = Context::default();
                    context.set_pixels_per_point(scale);
                    for action in [
                        TextAction::new("Cancel").toolbar(0.),
                        TextAction::new("Apply").primary().toolbar(0.),
                        TextAction::new("—").toolbar(50.),
                        TextAction::new("100%").toolbar(50.),
                        TextAction::new("12345%").toolbar(50.),
                    ] {
                        frame(&context, width, true, vec![], |ui| {
                            ui.visuals_mut().button_frame = framed;
                            if custom_style {
                                ui.style_mut().override_font_id =
                                    Some(egui::FontId::monospace(17.));
                                ui.spacing_mut().button_padding = egui::vec2(10., 5.);
                            }
                            let id = ui.next_auto_id();
                            let available = ui.available_rect_before_wrap();
                            let measured = action.width(ui);
                            assert_eq!(ui.next_auto_id(), id);
                            assert_eq!(ui.available_rect_before_wrap(), available);
                            let response = action.show(ui);
                            assert_eq!(response.id, id);
                            assert!(
                                (response.rect.width() - measured).abs() < 1e-5,
                                "measured {measured}, allocated {:?}: scale {scale}, framed {framed}, custom {custom_style}",
                                response.rect
                            );
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn semantic_name_is_independent_of_painted_label_and_reports_button_state() {
    let context = Context::default();
    for enabled in [true, false] {
        let (response, output) = frame(&context, 270., true, vec![], |ui| {
            TextAction::new("37%")
                .toolbar(50.)
                .enabled(enabled)
                .name("100%")
                .show(ui)
        });
        let node = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("100%"))
            .unwrap()
            .1;
        assert_eq!(node.role(), egui::accesskit::Role::Button);
        assert_eq!(node.is_disabled(), !enabled);
        let bounds = node.bounds().unwrap();
        assert_eq!(bounds.x0 as f32, response.rect.left());
        assert_eq!(bounds.x1 as f32, response.rect.right());
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == "37%")));
    }
}

#[test]
fn repeated_labels_keep_caller_scopes_and_direct_focus_order() {
    let context = Context::default();
    let render = |shared, events| {
        frame(&context, 315., true, events, |ui| {
            ["roll", "photo"].map(|scope| {
                ui.push_id(scope, |ui| {
                    let action = TextAction::new("Reset");
                    if shared {
                        action.show(ui)
                    } else {
                        direct(ui, action)
                    }
                })
                .inner
            })
        })
    };
    let (direct, _) = render(false, vec![]);
    let (shared, _) = render(true, vec![]);
    assert_ne!(shared[0].id, shared[1].id);
    assert_eq!(
        direct.map(|response| response.id),
        shared.clone().map(|response| response.id)
    );
    context.memory_mut(|memory| memory.request_focus(shared[0].id));
    render(true, key(egui::Key::Tab));
    assert_eq!(
        context.memory(|memory| memory.focused()),
        Some(shared[1].id)
    );
}

#[test]
fn mouse_and_keyboard_activation_respect_local_and_inherited_disable_transitions() {
    for inherited_enabled in [true, false] {
        for enabled in [true, false] {
            let context = Context::default();
            let render = |events| {
                frame(&context, 270., inherited_enabled, events, |ui| {
                    TextAction::new("Apply")
                        .enabled(enabled)
                        .primary()
                        .toolbar(0.)
                        .show(ui)
                })
            };
            let (response, _) = render(vec![]);
            render(pointer(response.rect.center(), true));
            let (clicked, _) = render(pointer(response.rect.center(), false));
            assert_eq!(clicked.clicked(), inherited_enabled && enabled);
            context.memory_mut(|memory| memory.request_focus(response.id));
            let (keyboard, _) = render(key(egui::Key::Space));
            assert_eq!(keyboard.clicked(), inherited_enabled && enabled);
        }
    }
    let context = Context::default();
    let render = |enabled, events| {
        frame(&context, 270., true, events, |ui| {
            TextAction::new("Apply")
                .enabled(enabled)
                .toolbar(0.)
                .show(ui)
        })
    };
    let (response, _) = render(true, vec![]);
    context.memory_mut(|memory| memory.request_focus(response.id));
    render(true, pointer(response.rect.center(), true));
    let (disabled, _) = render(false, pointer(response.rect.center(), false));
    assert!(!disabled.clicked());
    assert!(!render(false, key(egui::Key::Space)).0.clicked());
    assert!(!render(true, vec![]).0.clicked());
}
