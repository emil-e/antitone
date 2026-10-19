use super::{CONTROL_HEIGHT, frame, key, pointer};
use crate::editor::presentation::inspector_dropdown;
use egui::Context;

#[test]
fn inspector_dropdowns_fill_the_row_without_losing_names_values_or_disabled_state() {
    for scale in [1., 2.] {
        for width in [270., 315.] {
            for enabled in [true, false] {
                let context = Context::default();
                context.set_pixels_per_point(scale);
                let mut previous = None;
                for selected in [
                    "None",
                    "A loaded creative look with a name much longer than the inspector width",
                ] {
                    let render = || {
                        frame(&context, width, enabled, vec![], |ui| {
                            let available = ui.available_rect_before_wrap();
                            let spacing = ui.spacing().interact_size;
                            let response = inspector_dropdown(
                                ui,
                                "caller-selection",
                                "Look",
                                selected,
                                |ui| ui.label("Menu content"),
                            );
                            assert_eq!(ui.spacing().interact_size, spacing);
                            (available, response.response)
                        })
                    };
                    render();
                    let ((available, response), output) = render();
                    assert!((response.rect.left() - available.left()).abs() < 0.01);
                    assert!((response.rect.right() - available.right()).abs() < 0.01);
                    assert_eq!(response.rect.height(), CONTROL_HEIGHT);
                    assert_eq!(response.enabled(), enabled);
                    if let Some((id, rect)) = previous {
                        assert_eq!(response.id, id);
                        assert_eq!(response.rect, rect);
                    }
                    previous = Some((response.id, response.rect));
                    let node = &output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                        .iter()
                        .find(|(_, node)| node.role() == egui::accesskit::Role::ComboBox)
                        .unwrap()
                        .1;
                    assert_eq!(node.label(), Some("Look"));
                    assert_eq!(node.value(), Some(selected));
                    assert_eq!(node.is_disabled(), !enabled);
                }
            }
        }
    }
}

#[test]
fn inspector_dropdowns_keep_pointer_and_keyboard_menu_activation() {
    for enabled in [true, false] {
        for keyboard in [true, false] {
            let context = Context::default();
            let render = |events| {
                frame(&context, 270., enabled, events, |ui| {
                    inspector_dropdown(ui, "choice", "Look", "None", |ui| {
                        let _ = ui.selectable_label(true, "None");
                    })
                })
                .0
            };
            render(vec![]);
            let initial = render(vec![]);
            if keyboard {
                context.memory_mut(|memory| memory.request_focus(initial.response.id));
                render(key(egui::Key::Space));
            } else {
                render(pointer(initial.response.rect.center(), true));
                render(pointer(initial.response.rect.center(), false));
            }
            assert_eq!(render(vec![]).inner.is_some(), enabled);
            render(key(egui::Key::Escape));
            assert!(render(vec![]).inner.is_none());
        }
    }
}
