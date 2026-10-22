use super::{frame, key, pointer};
use crate::editor::{
    controls,
    icons::Icon,
    presentation::{
        CONTROL_HEIGHT, SessionToolGroup, ToolButton, ToolGroup, ToolIdentity, ToolShelf,
    },
};
use egui::{Context, FullOutput, Ui};

fn group(title: &str, busy: bool, apply_enabled: bool) -> ToolGroup<'_> {
    ToolGroup {
        identity: ToolIdentity {
            icon: if matches!(title, "Crop" | "Flat-field") {
                Icon::Crop
            } else {
                Icon::Pick
            },
            title,
        },
        busy,
        cancel_enabled: !busy,
        apply_enabled,
    }
}

#[test]
fn tool_warning_has_reserved_space_reason_semantics_and_unchanged_actions() {
    let reason = "No crop proposal: retained frame measurements are missing";
    for width in [270., 315.] {
        let context = Context::default();
        let ((reserved, rect, cancel, apply), output) =
            frame(&context, width, true, vec![], |ui| {
                let group = group("Auto-crop", false, false);
                let reserved = group.width_named(ui, "Apply");
                let response = group.show_warned(ui, "Apply", Some(reason));
                (
                    reserved,
                    response.response.rect,
                    response.inner.cancel,
                    response.inner.apply,
                )
            });
        assert!(rect.width() <= reserved + 1. && rect.right() <= width);
        assert!(cancel.enabled() && !apply.enabled());
        assert_eq!(node(&output, "Tool warning").description(), Some(reason));
        assert_eq!(
            node(&output, "Tool warning").role(),
            egui::accesskit::Role::Image
        );
        assert!(!cancel.rect.intersects(apply.rect));
        let ((plain_cancel, plain_apply), _) = frame(&context, width, true, vec![], |ui| {
            let actions = group("Auto-crop", false, false).show(ui).inner;
            (actions.cancel, actions.apply)
        });
        assert_eq!(
            cancel.id, plain_cancel.id,
            "cue changes must not move action focus to another widget"
        );
        assert_eq!(apply.id, plain_apply.id);
    }
}

fn node<'a>(output: &'a FullOutput, label: &str) -> &'a egui::accesskit::Node {
    &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| {
            node.role() != egui::accesskit::Role::TextRun
                && node.label().or_else(|| node.value()) == Some(label)
        })
        .unwrap_or_else(|| {
            panic!(
                "missing {label:?}; labels: {:?}",
                output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .map(|(_, node)| node.label().or_else(|| node.value()))
                    .collect::<Vec<_>>()
            )
        })
        .1
}

fn assert_icon_before_title(output: &FullOutput, title: &egui::accesskit::Node) {
    let title = title.bounds().unwrap();
    assert!(
        output.shapes.iter().any(|shape| {
            let bounds = shape
                .shape
                .visual_bounding_rect()
                .intersect(shape.clip_rect);
            bounds.is_positive()
                && f64::from(bounds.right()) <= title.x0
                && f64::from(bounds.bottom()) > title.y0
                && f64::from(bounds.top()) < title.y1
        }),
        "tool icon must be painted before its title"
    );
}

#[test]
fn stage_tool_shelf_distinguishes_commands_active_tools_and_changed_state() {
    let context = Context::default();
    let ((turn, crop, dust), output) = frame(&context, 315., true, vec![], |ui| {
        ToolShelf::show(ui, |ui| {
            let turn = ToolButton::command(Icon::TurnLeft, "Turn left", "Turn the photo.").show(ui);
            ui.separator();
            let crop =
                ToolButton::tool(Icon::Crop, "Crop & straighten", "Edit crop.", true).show(ui);
            let dust = ToolButton::tool(Icon::Dust, "Dust repair", "Remove dust.", false)
                .changed(true)
                .show(ui);
            (turn, crop, dust)
        })
    });
    assert!(turn.enabled());
    assert!(crop.enabled());
    assert!(dust.enabled());
    assert_eq!(node(&output, "Crop & straighten").is_selected(), Some(true));
    assert_eq!(turn.rect.height(), 28.);
    assert_eq!(crop.rect.height(), 28.);
    assert_eq!(dust.rect.height(), 28.);
}

#[test]
fn session_group_measures_and_returns_one_right_aligned_done_action() {
    let context = Context::default();
    for enabled in [true, false] {
        frame(&context, 315., true, vec![], |ui| {
            let group = SessionToolGroup {
                identity: ToolIdentity {
                    icon: Icon::Dust,
                    title: "Dust cleanup",
                },
                busy: !enabled,
                done_enabled: enabled,
            };
            let next = ui.next_auto_id();
            let measured = group.width(ui);
            assert_eq!(ui.next_auto_id(), next);
            let response = group.show(ui);
            assert!((response.response.rect.width() - measured).abs() < 1e-5);
            assert_eq!(response.response.rect.height(), 28.);
            assert_eq!(response.inner.enabled(), enabled);
        });
    }
}

#[test]
fn viewer_groups_keep_actions_ordered_contained_and_stable_across_frames() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for enabled in [true, false] {
                for title in [
                    "Crop",
                    "Dmin",
                    "Dmax",
                    "Gamma neutral",
                    "WB neutral",
                    "Flat-field",
                ] {
                    let context = Context::default();
                    context.set_pixels_per_point(scale);
                    frame(&context, width, enabled, vec![], |_| ());
                    let render = || {
                        frame(&context, width, enabled, vec![], |ui| {
                            ui.spacing_mut().item_spacing.x = 4.;
                            ui.push_id("viewer", |ui| {
                                let response = ui
                                    .with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| group(title, false, title != "Dmin").show(ui),
                                    )
                                    .inner;
                                (response, ui.button("Following"))
                            })
                            .inner
                        })
                    };
                    let ((before, following_before), _) = render();
                    let ((after, following_after), output) = render();
                    assert_eq!(after.response.rect, before.response.rect);
                    assert_eq!(after.response.id, before.response.id);
                    assert_eq!(after.inner.cancel.rect, before.inner.cancel.rect);
                    assert_eq!(after.inner.apply.rect, before.inner.apply.rect);
                    assert_eq!(after.inner.cancel.id, before.inner.cancel.id);
                    assert_eq!(after.inner.apply.id, before.inner.apply.id);
                    assert_eq!(following_after.id, following_before.id);
                    assert_ne!(after.inner.cancel.id, after.inner.apply.id);
                    assert_ne!(after.inner.apply.id, following_after.id);
                    assert_eq!(after.inner.apply.rect.height(), CONTROL_HEIGHT);
                    assert_eq!(after.response.rect.height(), CONTROL_HEIGHT);
                    assert_eq!(after.inner.cancel.enabled(), enabled);
                    assert_eq!(after.inner.apply.enabled(), enabled && title != "Dmin");
                    assert!(after.response.rect.contains_rect(after.inner.cancel.rect));
                    assert!(after.response.rect.contains_rect(after.inner.apply.rect));
                    assert!(after.inner.cancel.rect.right() <= after.inner.apply.rect.left());
                    let title_node = node(&output, &format!("Active tool: {title}"));
                    assert_icon_before_title(&output, title_node);
                    let title_bounds = title_node.bounds().unwrap();
                    assert!(title_bounds.x1 < f64::from(after.inner.cancel.rect.left()));
                    let fill = |response: &egui::Response| {
                        output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Rect(rect) if rect.rect == response.rect => {
                                    Some(rect.fill)
                                }
                                _ => None,
                            })
                            .unwrap()
                    };
                    assert_ne!(fill(&after.inner.apply), fill(&after.inner.cancel));
                    for (name, response) in [
                        ("Cancel", &after.inner.cancel),
                        ("Apply", &after.inner.apply),
                    ] {
                        let action = node(&output, name);
                        assert_eq!(action.role(), egui::accesskit::Role::Button);
                        assert_eq!(action.is_disabled(), !response.enabled());
                    }
                }
            }
        }
    }
}

#[test]
fn complete_width_matches_allocation_without_changing_ids_or_layout() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for custom in [true, false] {
                for framed in [true, false] {
                    for busy in [true, false] {
                        let context = Context::default();
                        context.set_pixels_per_point(scale);
                        frame(&context, width, true, vec![], |ui| {
                            ui.spacing_mut().item_spacing.x = if custom { 7. } else { 4. };
                            ui.visuals_mut().button_frame = framed;
                            if custom {
                                ui.style_mut().override_font_id =
                                    Some(egui::FontId::monospace(17.));
                                ui.spacing_mut().button_padding = egui::vec2(10., 5.);
                            }
                            let group = group("Gamma neutral", busy, !busy);
                            let id = ui.next_auto_id();
                            let available = ui.available_rect_before_wrap();
                            let measured = group.width(ui);
                            assert_eq!(ui.next_auto_id(), id);
                            assert_eq!(ui.available_rect_before_wrap(), available);
                            let response = group.show(ui);
                            assert!(
                                (response.response.rect.width() - measured).abs() < 1e-5,
                                "measured {measured}, allocated {:?}, scale {scale}, custom {custom}, framed {framed}, busy {busy}",
                                response.response.rect
                            );
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn busy_identity_changes_semantics_without_moving_actions_or_frame() {
    let context = Context::default();
    let render = |busy| {
        frame(&context, 270., true, vec![], |ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            group("Dmin", busy, !busy).show(ui)
        })
    };
    let (idle, idle_output) = render(false);
    let (busy, busy_output) = render(true);
    assert_eq!(idle.response.rect, busy.response.rect);
    assert_eq!(idle.inner.cancel.rect, busy.inner.cancel.rect);
    assert_eq!(idle.inner.apply.rect, busy.inner.apply.rect);
    assert_eq!(
        node(&idle_output, "Active tool: Dmin").bounds(),
        node(&busy_output, "Active tool: Dmin · Working").bounds()
    );
    for name in ["Cancel", "Apply"] {
        assert_eq!(
            node(&busy_output, name).role(),
            egui::accesskit::Role::Button
        );
        assert!(node(&busy_output, name).is_disabled());
        assert!(!node(&idle_output, name).is_disabled());
    }
    // Busy affects identity only; availability is explicitly supplied by the controller.
    assert!(
        frame(&context, 270., true, vec![], |ui| {
            group("Dmin", true, true).show(ui).inner.apply.enabled()
        })
        .0
    );
}

#[test]
fn group_responses_keep_mouse_keyboard_focus_and_disabled_transition_behavior() {
    for inherited in [true, false] {
        for apply_enabled in [true, false] {
            let context = Context::default();
            let render = |events| {
                frame(&context, 270., inherited, events, |ui| {
                    group("Crop", false, apply_enabled).show(ui).inner
                })
            };
            let (actions, _) = render(vec![]);
            render(pointer(actions.apply.rect.center(), true));
            assert_eq!(
                render(pointer(actions.apply.rect.center(), false))
                    .0
                    .apply
                    .clicked(),
                inherited && apply_enabled
            );
            context.memory_mut(|memory| memory.request_focus(actions.apply.id));
            assert_eq!(
                render(key(egui::Key::Space)).0.apply.clicked(),
                inherited && apply_enabled
            );
            if inherited && apply_enabled {
                context.memory_mut(|memory| memory.request_focus(actions.cancel.id));
                render(key(egui::Key::Tab));
                assert_eq!(
                    context.memory(|memory| memory.focused()),
                    Some(actions.apply.id)
                );
            }
        }
    }
    let context = Context::default();
    let render = |busy, events| {
        frame(&context, 270., true, events, |ui| {
            group("Dmin", busy, !busy).show(ui).inner
        })
    };
    let (actions, _) = render(false, vec![]);
    render(false, pointer(actions.cancel.rect.center(), true));
    assert!(
        !render(true, pointer(actions.cancel.rect.center(), false))
            .0
            .cancel
            .clicked()
    );
    context.memory_mut(|memory| memory.request_focus(actions.apply.id));
    assert!(!render(true, key(egui::Key::Space)).0.apply.clicked());
    let (restored, _) = render(false, vec![]);
    assert!(!restored.cancel.clicked() && !restored.apply.clicked());
}

#[test]
fn inspector_titles_resets_and_body_actions_fit_and_expose_supplied_availability() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for enabled in [true, false] {
                for reset in [None, Some(false), Some(true)] {
                    let context = Context::default();
                    context.set_pixels_per_point(scale);
                    frame(&context, width, enabled, vec![], |_| ());
                    let title = if reset.is_none() {
                        "Auto-crop"
                    } else {
                        "Crop & straighten"
                    };
                    let render = || {
                        frame(&context, width, enabled, vec![], |ui| {
                            ui.push_id("inspector", |ui| {
                                ui.vertical(|ui| {
                                    controls::active_tool(
                                        ui,
                                        Icon::Crop,
                                        title,
                                        reset,
                                        |ui: &mut Ui, reset_clicked| {
                                            assert!(!reset_clicked);
                                            ui.button("Body action")
                                        },
                                    )
                                })
                                .inner
                            })
                            .inner
                        })
                    };
                    let (before, _) = render();
                    let (after, output) = render();
                    assert_eq!(after.rect, before.rect);
                    assert_eq!(after.id, before.id);
                    assert_eq!(after.enabled(), enabled);
                    let title_node = node(&output, title);
                    assert_eq!(title_node.role(), egui::accesskit::Role::Label);
                    assert_icon_before_title(&output, title_node);
                    let title_bounds = title_node.bounds().unwrap();
                    let has_reset = output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                        .iter()
                        .any(|(_, node)| node.label() == Some("Reset draft"));
                    assert_eq!(has_reset, reset.is_some());
                    assert!(title_bounds.y1 <= f64::from(after.rect.top()));
                    if let Some(reset_enabled) = reset {
                        let reset_node = node(&output, "Reset draft");
                        assert_eq!(reset_node.is_disabled(), !enabled || !reset_enabled);
                        let reset_bounds = reset_node.bounds().unwrap();
                        assert!(title_bounds.x1 <= reset_bounds.x0);
                        assert!(reset_bounds.y1 <= f64::from(after.rect.top()));
                    }
                }
            }
        }
    }
}

#[test]
fn inspector_header_result_reaches_body_without_owning_state_or_reset_scope() {
    let context = Context::default();
    let (response, _) = frame(&context, 315., true, vec![], |ui| {
        ToolIdentity {
            icon: Icon::Crop,
            title: "Crop & straighten",
        }
        .inspector(
            ui,
            |ui| ui.button("Caller header"),
            |ui, response| {
                ui.label("Caller body");
                response
            },
        )
    });
    assert!(!response.clicked());
    let render = |events| {
        frame(&context, 315., true, events, |ui| {
            controls::active_tool(
                ui,
                Icon::Crop,
                "Crop & straighten",
                Some(true),
                |_, reset| reset,
            )
        })
    };
    let (_, output) = render(vec![]);
    let bounds = node(&output, "Reset draft").bounds().unwrap();
    let position = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    );
    render(pointer(position, true));
    assert!(render(pointer(position, false)).0);
}
