use super::{frame, key, pointer};
use crate::editor::{
    icons::{self, Icon},
    presentation::{SectionHeader, collect_expansion, labelled_row, restore_expansion, section},
};
use egui::{Context, Event, Response, Ui};
use std::collections::BTreeMap;

fn contents(ui: &mut Ui, numeric: bool) -> Vec<Response> {
    if numeric {
        let reset = icons::button(ui, Icon::Reset, true, "Reset value", "Reset value");
        ui.label("EV");
        let mut value = 1.23456789;
        let numeric = ui
            .push_id("numeric", |ui| {
                ui.add_sized(
                    [85., ui.spacing().interact_size.y],
                    egui::DragValue::new(&mut value).update_while_editing(false),
                )
            })
            .inner;
        vec![reset, numeric]
    } else {
        vec![icons::button(
            ui,
            Icon::Pick,
            true,
            "Pick value",
            "Pick value",
        )]
    }
}

fn direct_row(
    ui: &mut Ui,
    label: &str,
    help: Option<&str>,
    numeric: bool,
) -> egui::InnerResponse<Vec<Response>> {
    icons::row(ui, |ui| {
        let label = ui.label(label);
        if let Some(help) = help {
            label.on_hover_text(help);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            contents(ui, numeric)
        })
        .inner
    })
}

#[test]
fn labelled_rows_preserve_direct_layout_paint_ids_and_inherited_disable() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for enabled in [true, false] {
                for numeric in [true, false] {
                    for help in [None, Some("Caller meaning")] {
                        let context = Context::default();
                        context.set_pixels_per_point(scale);
                        frame(&context, width, enabled, vec![], |_| ());
                        let label = if numeric {
                            "Exposure"
                        } else {
                            "Dmin (film base)"
                        };
                        let render = |shared| {
                            frame(&context, width, enabled, vec![], |ui| {
                                ui.push_id("caller", |ui| {
                                    let row = if shared {
                                        labelled_row(ui, label, help, |ui| contents(ui, numeric))
                                    } else {
                                        direct_row(ui, label, help, numeric)
                                    };
                                    (row, ui.button("Following"))
                                })
                                .inner
                            })
                        };
                        let ((before, following_before), before_output) = render(false);
                        let ((after, following_after), after_output) = render(true);
                        assert_eq!(after.response.rect, before.response.rect);
                        assert_eq!(after.response.id, before.response.id);
                        assert_eq!(after.response.rect.height(), 28.);
                        for (before, after) in before.inner.iter().zip(&after.inner) {
                            assert_eq!(after.rect, before.rect);
                            assert_eq!(after.id, before.id);
                            assert_eq!(after.enabled(), enabled);
                        }
                        assert_eq!(following_after.id, following_before.id);
                        assert_eq!(after_output.shapes, before_output.shapes);
                    }
                }
            }
        }
    }
}

#[test]
fn labelled_rows_return_caller_responses_and_keep_focus_and_activation() {
    for enabled in [true, false] {
        let context = Context::default();
        let render = |events| {
            frame(&context, 270., enabled, events, |ui| {
                labelled_row(ui, "Caller label", None, |ui| {
                    [ui.button("First"), ui.button("Second")]
                })
                .inner
            })
        };
        let (responses, _) = render(vec![]);
        render(pointer(responses[0].rect.center(), true));
        assert_eq!(
            render(pointer(responses[0].rect.center(), false)).0[0].clicked(),
            enabled
        );
        context.memory_mut(|memory| memory.request_focus(responses[0].id));
        assert_eq!(render(key(egui::Key::Space)).0[0].clicked(), enabled);
        if enabled {
            render(key(egui::Key::Tab));
            assert_eq!(
                context.memory(|memory| memory.focused()),
                Some(responses[1].id)
            );
        }
    }
}

fn section_frame(
    context: &Context,
    width: f32,
    events: Vec<Event>,
    preferences: &mut BTreeMap<String, bool>,
    pinned: bool,
) -> (Option<Response>, bool, egui::FullOutput) {
    context.enable_accesskit();
    let mut body = None;
    let mut reset = false;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 300.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            restore_expansion(ui.ctx(), preferences.clone());
            (body, reset) = section(
                ui,
                "caller-section",
                "Caller section",
                SectionHeader {
                    summary: "Caller summary",
                    changed: true,
                    reset: Some(true),
                    default_open: if pinned { None } else { Some(true) },
                },
                |ui| ui.button("Caller body"),
            );
            *preferences = collect_expansion(ui.ctx());
        },
    );
    output.textures_delta.clear();
    (body, reset, output)
}

#[test]
fn sections_keep_pinned_expansion_and_independent_reset_without_domain_fixtures() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            let context = Context::default();
            context.set_pixels_per_point(scale);
            let mut preferences = BTreeMap::from([("caller-section".into(), false)]);
            section_frame(&context, width, vec![], &mut preferences, false);
            let (body, reset, output) =
                section_frame(&context, width, vec![], &mut preferences, false);
            assert!(body.is_none() && !reset);
            assert_eq!(preferences.get("caller-section"), Some(&false));
            let header = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Caller section"))
                .unwrap();
            assert_eq!(header.1.role(), egui::accesskit::Role::Button);
            assert_eq!(header.1.is_expanded(), Some(false));
            let bounds = header.1.bounds().unwrap();
            let header_pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.) as f32,
                ((bounds.y0 + bounds.y1) / 2.) as f32,
            );
            let reset = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Reset Caller section"))
                .unwrap();
            let bounds = reset.1.bounds().unwrap();
            let reset_pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.) as f32,
                ((bounds.y0 + bounds.y1) / 2.) as f32,
            );
            section_frame(
                &context,
                width,
                pointer(reset_pos, true),
                &mut preferences,
                false,
            );
            assert!(
                section_frame(
                    &context,
                    width,
                    pointer(reset_pos, false),
                    &mut preferences,
                    false
                )
                .1
            );
            assert_eq!(preferences.get("caller-section"), Some(&false));
            let (pinned, _, output) =
                section_frame(&context, width, vec![], &mut preferences, true);
            assert!(pinned.is_some());
            let title = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| {
                    node.role() == egui::accesskit::Role::Label
                        && node.value() == Some("Caller section")
                })
                .unwrap();
            assert_eq!(title.1.is_expanded(), None);
            assert_eq!(preferences.get("caller-section"), Some(&false));
            assert!(
                section_frame(&context, width, vec![], &mut preferences, false)
                    .0
                    .is_none()
            );
            section_frame(
                &context,
                width,
                pointer(header_pos, true),
                &mut preferences,
                false,
            );
            let (opened, _, _) = section_frame(
                &context,
                width,
                pointer(header_pos, false),
                &mut preferences,
                false,
            );
            assert!(
                opened.is_some(),
                "scale {scale}, width {width}, header {header_pos:?}, preferences {preferences:?}"
            );
            let id = opened.unwrap().id;
            assert_eq!(preferences.get("caller-section"), Some(&true));
            let fresh = Context::default();
            fresh.set_pixels_per_point(scale);
            let (restored, _, _) = section_frame(&fresh, width, vec![], &mut preferences, false);
            assert_eq!(restored.unwrap().id, id);
        }
    }
}
