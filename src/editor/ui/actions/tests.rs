use super::*;
use crate::editor::ui::test_support::{frame, key, pointer};
use egui::Context;

#[test]
fn text_actions_fit_their_labels_and_keep_ids_and_supplied_availability() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for inherited_enabled in [true, false] {
                for action in [
                    TextAction::new("Choose…").help("Explicit opt-in"),
                    TextAction::new("Set up…"),
                    TextAction::new("Cancel"),
                    TextAction::new("Discard"),
                    TextAction::new("Save"),
                    TextAction::new("Replace"),
                    TextAction::new("Analyze roll").primary(),
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
                    let draw = || {
                        frame(&context, width, inherited_enabled, vec![], |ui| {
                            action.show(ui)
                        })
                    };
                    let (before, _) = draw();
                    let (after, output) = draw();
                    assert_eq!(after.id, before.id);
                    assert_eq!(after.rect, before.rect);
                    assert_eq!(after.enabled(), inherited_enabled && action.enabled);
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == action.label => {
                                Some(text)
                            }
                            _ => None,
                        })
                        .unwrap();
                    let bounds = text.galley.rect.translate(text.pos.to_vec2());
                    assert!(after.rect.expand(0.5).contains_rect(bounds));
                    if let Some(min_width) = action.toolbar_width {
                        assert_eq!(after.rect.height(), CONTROL_HEIGHT);
                        assert!(after.rect.width() >= min_width);
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
fn repeated_labels_keep_caller_scopes_and_focus_order() {
    let context = Context::default();
    let render = |events| {
        frame(&context, 315., true, events, |ui| {
            ["roll", "photo"].map(|scope| {
                ui.push_id(scope, |ui| TextAction::new("Reset").show(ui))
                    .inner
            })
        })
    };
    let (before, _) = render(vec![]);
    let (after, _) = render(vec![]);
    assert_ne!(after[0].id, after[1].id);
    assert_eq!(
        before.map(|response| response.id),
        after.clone().map(|response| response.id)
    );
    context.memory_mut(|memory| memory.request_focus(after[0].id));
    render(key(egui::Key::Tab));
    assert_eq!(context.memory(|memory| memory.focused()), Some(after[1].id));
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
