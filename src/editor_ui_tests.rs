#[path = "editor_ui_dropdown_tests.rs"]
mod dropdowns;
#[path = "editor_ui_row_tests.rs"]
mod rows;
#[path = "editor_ui_selection_tests.rs"]
mod selection;
#[path = "editor_tool_group_tests.rs"]
mod tool_groups;
#[path = "editor_ui_typography_tests.rs"]
mod typography;

use super::{CONTROL_HEIGHT, INSCRIPTION, TextAction, app_header, configure_branding};
use egui::{Context, Event, FullOutput, Modifiers, Pos2, Rect, Response, Ui};

pub(super) fn frame<R>(
    context: &Context,
    width: f32,
    enabled: bool,
    events: Vec<Event>,
    mut draw: impl FnMut(&mut Ui) -> R,
) -> (R, FullOutput) {
    super::configure_fonts(context);
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

#[test]
fn rotation_cursor_bitmaps_keep_the_hotspot_contrast_and_bounded_storage() {
    let context = Context::default();
    let id = egui::Id::new("rotation-cursor");
    for step in 0..32 {
        let angle = step as f32 * std::f32::consts::TAU / 32.;
        let image = super::icons::rotation_cursor(&context, id, angle);
        assert_eq!(image.size, [31, 31]);
        assert_eq!(image.hotspot, [15, 15]);
        assert_eq!(image.rgba.len(), 31 * 31 * 4);
        assert_eq!(&image.rgba[(15 * 31 + 15) * 4..][..4], &[255; 4]);
        let (pixels, remainder) = image.rgba.as_chunks::<4>();
        assert!(remainder.is_empty());
        assert!(pixels.iter().any(|pixel| pixel[0] == 0 && pixel[3] > 0));
        for (index, pixel) in pixels.iter().enumerate() {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            if index % 31 == 0 || index % 31 == 30 || index / 31 == 0 || index / 31 == 30 {
                assert_eq!(pixel[3], 0, "cursor clipped at angle {angle}");
            }
        }
        let reused = super::icons::rotation_cursor(&context, id, angle);
        assert!(std::sync::Arc::ptr_eq(&image.rgba, &reused.rgba));
    }
    let original = super::icons::rotation_cursor(&context, id, 0.);
    let turned = super::icons::rotation_cursor(&context, id, std::f32::consts::FRAC_PI_2);
    assert!(!std::sync::Arc::ptr_eq(&original.rgba, &turned.rgba));
    for y in 0..31 {
        for x in 0..31 {
            let a = original.rgba[(y * 31 + x) * 4 + 3];
            let b = turned.rgba[(x * 31 + 30 - y) * 4 + 3];
            assert!(a.abs_diff(b) <= 1);
        }
    }
    frame(&context, 200., true, vec![], |ui| {
        let before = ui.next_auto_id();
        super::icons::rotation_cursor(ui.ctx(), id, 0.);
        assert_eq!(ui.next_auto_id(), before);
    });
}

pub(in crate::editor) fn key(key: egui::Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

pub(in crate::editor) fn pointer(position: Pos2, pressed: bool) -> Vec<Event> {
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
fn brand_assets_have_the_expected_sizes_and_self_contained_wordmarks() {
    let header = image::load_from_memory(include_bytes!("../assets/branding/wordmark-header.png"))
        .unwrap()
        .into_rgba8();
    assert_eq!(header.dimensions(), (472, 64));
    assert!(
        header
            .pixels()
            .any(|pixel| pixel.0 == INSCRIPTION.to_array())
    );
    assert!(header.pixels().any(|pixel| pixel.0[3] == 0));
    for svg in [
        include_str!("../assets/branding/wordmark-amber.svg"),
        include_str!("../assets/branding/wordmark-density.svg"),
    ] {
        assert!(svg.contains("<path"));
        assert!(!svg.contains("<text"));
        assert!(!svg.contains("href="));
    }
    let ico = include_bytes!("../assets/branding/icon.ico");
    assert_eq!(&ico[..6], &[0, 0, 1, 0, 7, 0]);
    let mut next_offset = 6 + 7 * 16;
    for (index, size) in [16, 24, 32, 48, 64, 128, 256].into_iter().enumerate() {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
        assert_eq!(entry[0] as u32, size % 256);
        assert_eq!(entry[1], entry[0]);
        let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        assert_eq!(offset, next_offset);
        let image = image::load_from_memory(&ico[offset..offset + length]).unwrap();
        assert_eq!((image.width(), image.height()), (size, size));
        next_offset += length;
    }
    assert_eq!(next_offset, ico.len());
    let icns = include_bytes!("../assets/branding/icon.icns");
    assert_eq!(&icns[..4], b"icns");
    assert_eq!(
        u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize,
        icns.len()
    );
}

#[test]
fn shared_components_inherit_interaction_colours_without_tinting_photographic_surfaces() {
    for accent in [INSCRIPTION, egui::Color32::from_rgb(71, 143, 207)] {
        let context = Context::default();
        super::configure_fonts(&context);
        let mut rectangles = None;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 320.))),
                ..Default::default()
            },
            |ui| {
                configure_branding(ui);
                assert_eq!(ui.visuals().selection.stroke.color, INSCRIPTION);
                assert_eq!(
                    ui.visuals().selection.bg_fill,
                    INSCRIPTION.gamma_multiply(0.25)
                );
                assert_eq!(ui.visuals().hyperlink_color, INSCRIPTION);
                // A second accent detects components that hard-code the current brand colour.
                ui.visuals_mut().selection.stroke.color = accent;
                ui.visuals_mut().selection.bg_fill = accent.gamma_multiply(0.25);
                let action = TextAction::new("Apply").primary().show(ui);
                let mut selected = 1;
                super::stage_tabs(ui, &mut selected);
                super::ToolIdentity {
                    icon: super::Icon::Pick,
                    title: "Dmin",
                }
                .inspector(ui, |_| (), |ui, ()| ui.label("Tool details"));
                let layout = super::ContactSheetLayout::new(100., false);
                let (id, cell) = ui.allocate_space(layout.size());
                let card = layout.begin(ui, cell.min, id, true, false, true);
                rectangles = Some((action.rect, card.response.rect));
            },
        );
        output.textures_delta.clear();
        let (action, card) = rectangles.unwrap();
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.rect == action && rect.fill == accent.gamma_multiply(0.25))));
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill == accent.gamma_multiply(0.25).linear_multiply(0.15))));
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::LineSegment { stroke, .. } if stroke.width == 2. && stroke.color == accent)));
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.rect == card && rect.stroke.color == accent
                && rect.fill.r() == rect.fill.g() && rect.fill.g() == rect.fill.b())));
    }
}

#[test]
fn app_header_keeps_the_wordmark_size_neutral_theme_and_menu_focus_at_both_scales() {
    for scale in [1., 2.] {
        for width in [320., 720., 1100.] {
            let context = Context::default();
            context.enable_accesskit();
            let mut buttons = None;
            let mut draw = |events| {
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 200.))),
                    events,
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                let mut output = context.run_ui(input, |ui| {
                    configure_branding(ui);
                    assert_eq!(ui.ctx().pixels_per_point(), scale);
                    assert_eq!(ui.visuals().selection.stroke.color, INSCRIPTION);
                    let surface = ui.visuals().panel_fill;
                    assert!(surface.r().abs_diff(surface.g()) <= 4);
                    assert!(surface.g().abs_diff(surface.b()) <= 4);
                    app_header(ui, |ui| {
                        let file = ui.menu_button("File", |_| ());
                        let export = ui.menu_button("Export", |_| ());
                        let name = ui.add(
                            egui::Label::new("A very long project filename.antitone.json")
                                .truncate(),
                        );
                        assert!(
                            name.rect.right() <= width,
                            "name {:?}, screen {width}, scale {scale}",
                            name.rect
                        );
                        buttons = Some([file.response, export.response]);
                    });
                });
                output.textures_delta.clear();
                output
            };
            let output = draw(vec![]);
            let update = output.platform_output.accesskit_update.as_ref().unwrap();
            let mark = &update
                .nodes
                .iter()
                .find(|(_, node)| node.value() == Some("Antitone"))
                .unwrap()
                .1;
            assert_eq!(mark.role(), egui::accesskit::Role::Label);
            let bounds = mark.bounds().unwrap();
            assert_eq!(bounds.x1 - bounds.x0, 118.);
            assert_eq!(bounds.y1 - bounds.y0, 16.);
            assert_eq!(bounds.x0, 12.);
            assert!(((bounds.y0 + bounds.y1) / 2. - 16.).abs() < 1.);
            let mark_before = bounds;
            let after = draw(key(egui::Key::Tab));
            let mark_after = after
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.value() == Some("Antitone"))
                .map(|(_, node)| node.bounds().unwrap());
            if let Some(bounds) = mark_after {
                assert_eq!(bounds, mark_before);
            }
            drop(draw);
            let buttons = buttons.unwrap();
            assert_eq!(
                context.memory(|memory| memory.focused()),
                Some(buttons[0].id)
            );
            assert!(buttons.iter().all(|button| button.rect.bottom() <= 32.));
        }
    }
}

#[test]
fn text_actions_fit_their_labels_and_keep_ids_and_supplied_availability() {
    for scale in [1., 2.] {
        for width in [315., 270.] {
            for inherited_enabled in [true, false] {
                for action in [
                    TextAction::new("Use adjustable response").help("Explicit opt-in"),
                    TextAction::new("Choose reference RAW…"),
                    TextAction::new("Edit area…"),
                    TextAction::new("Clear"),
                    TextAction::new("Cancel"),
                    TextAction::new("Discard"),
                    TextAction::new("Save"),
                    TextAction::new("Replace"),
                    TextAction::new("Analyze roll").primary(),
                    TextAction::new("Edit shared picture area…"),
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
fn natural_text_actions_use_egui_button_sizing_and_caller_ids() {
    for enabled in [false, true] {
        let context = Context::default();
        frame(&context, 315., enabled, vec![], |_| ());
        let (button, _) = frame(&context, 315., enabled, vec![], |ui| ui.button("Save"));
        let (action, _) = frame(&context, 315., enabled, vec![], |ui| {
            TextAction::new("Save").show(ui)
        });
        assert_eq!(action.rect, button.rect);
        assert_eq!(action.id, button.id);
        assert_eq!(action.enabled(), button.enabled());
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
