use super::*;
use crate::editor::presentation::tests::{frame, key, pointer};
use egui::{Context, Event, FullOutput, Pos2};

#[derive(Clone, Copy)]
enum Content {
    Empty,
    Loading,
    Error,
    Camera,
}

fn camera_image(ui: &Ui, image_rect: Rect) {
    let dimensions = egui::vec2(60., 30.);
    let scale = (image_rect.width() / dimensions.x).min(image_rect.height() / dimensions.y);
    let rect = Rect::from_center_size(image_rect.center(), dimensions * scale);
    ui.painter()
        .with_clip_rect(image_rect.intersect(ui.clip_rect()))
        .image(
            egui::TextureId::Managed(123),
            rect,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
            Color32::WHITE,
        );
}

#[derive(Clone, Copy)]
struct CardFixture {
    height: f32,
    enabled: bool,
    origin: Pos2,
    selected: bool,
    content: Content,
}

fn render(
    context: &Context,
    fixture: CardFixture,
    events: Vec<Event>,
) -> ((Response, Rect, Response), FullOutput) {
    let CardFixture {
        height,
        enabled,
        origin,
        selected,
        content,
    } = fixture;
    frame(context, 315., enabled, events, |ui| {
        let layout = ContactSheetLayout::new(height - 12., height < 90.);
        let id = ui.id().with("caller-card");
        let card = layout.begin(ui, origin, id, selected, false, true);
        match content {
            Content::Empty => (),
            Content::Loading => card.loading(ui),
            Content::Error => card.error(ui),
            Content::Camera => {
                camera_image(ui, card.image_rect);
                card.camera_badge(ui);
            }
        }
        card.caption(ui, "01.ARW");
        (card.response, card.image_rect, ui.button("Following"))
    })
}

#[test]
fn contextual_status_combines_activity_and_failure_without_hiding_reasons() {
    use crate::editor::presentation::{ContextualStatus, StatusLevel};
    let activity = ContextualStatus::new(StatusLevel::Activity, "Preparing thumbnail");
    let warning = ContextualStatus::new(StatusLevel::Warning, "No crop proposal");
    let combined = activity.clone().combine(warning);
    assert_eq!(combined.level, StatusLevel::Warning);
    assert!(
        combined.reason.contains("Preparing thumbnail")
            && combined.reason.contains("No crop proposal")
    );
    assert_eq!(activity.clone().combine(activity.clone()), activity);
    for (status, label) in [
        (activity, "Photo updating"),
        (combined, "Photo warning"),
        (
            ContextualStatus::new(StatusLevel::Error, "Repair has no donor"),
            "Photo error",
        ),
    ] {
        let context = Context::default();
        context.enable_accesskit();
        let ((image, response), output) = frame(&context, 315., true, vec![], |ui| {
            let card = ContactSheetLayout::new(32., true).begin(
                ui,
                egui::pos2(20., 10.),
                ui.id().with("status-photo"),
                false,
                false,
                true,
            );
            card.caption(ui, "01.ARW");
            (card.image_rect, card.status(ui, &status))
        });
        assert!(image.contains_rect(response.rect));
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(nodes.iter().any(|(_, node)| node.label() == Some(label)
            && node.description() == Some(status.reason.as_str())));
        assert!(nodes.iter().any(|(_, node)| node.label() == Some("01.ARW")
            && node.description() == Some(status.reason.as_str())));
    }
}

#[test]
fn warning_cues_fit_compact_cards_explain_the_photo_and_do_not_steal_selection() {
    for height in [32., 64., 140.] {
        let context = Context::default();
        let reason = "No crop proposal: four measured edges are required";
        let draw = |events| {
            frame(&context, 315., true, events, |ui| {
                let layout = ContactSheetLayout::new(height, true);
                let card = layout.begin(
                    ui,
                    egui::pos2(20., 10.),
                    ui.id().with("warned-photo"),
                    true,
                    true,
                    true,
                );
                card.caption(ui, "01.ARW");
                let warning = card.status(
                    ui,
                    &crate::editor::presentation::ContextualStatus::new(
                        crate::editor::presentation::StatusLevel::Warning,
                        reason,
                    ),
                );
                (card.response, card.image_rect, warning)
            })
        };
        let ((_, image, warning), output) = draw(vec![]);
        assert!(image.contains_rect(warning.rect));
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Photo warning")
                    && n.description() == Some(reason))
        );
        assert!(nodes.iter().any(|(_, n)| {
            n.label() == Some("01.ARW")
                && n.description()
                    .is_some_and(|d| d.contains("Active photo") && d.contains(reason))
        }));
        let point = warning.rect.center();
        draw(pointer(point, true));
        assert!(
            draw(pointer(point, false)).0.0.clicked(),
            "warning hover must not intercept photo selection"
        );
    }
}

#[test]
fn cards_keep_geometry_and_ids_across_focus_changes() {
    for scale in [1., 2.] {
        for height in [52., 80., 140., 240.] {
            for origin in [
                egui::pos2(10., 10.),
                egui::pos2(-35.25, 10.),
                egui::pos2(270.25, 10.),
            ] {
                for selected in [false, true] {
                    for content in [
                        Content::Empty,
                        Content::Loading,
                        Content::Error,
                        Content::Camera,
                    ] {
                        for enabled in [false, true] {
                            let context = Context::default();
                            context.set_pixels_per_point(scale);
                            frame(&context, 315., enabled, vec![], |_| ());
                            let fixture = CardFixture {
                                height,
                                enabled,
                                origin,
                                selected,
                                content,
                            };
                            for focused in [false, true] {
                                let (before, before_output) = render(&context, fixture, vec![]);
                                if focused {
                                    context.memory_mut(|memory| memory.request_focus(before.0.id));
                                }
                                let (after, after_output) = render(&context, fixture, vec![]);
                                assert_eq!(after.0.id, before.0.id);
                                assert_eq!(after.0.rect, before.0.rect);
                                assert_eq!(after.0.enabled(), enabled);
                                assert_eq!(after.0.has_focus(), enabled && focused);
                                assert_eq!(after.1, before.1);
                                assert!(after.0.rect.contains_rect(after.1));
                                assert_eq!(after.2.id, before.2.id);
                                assert_eq!(after.2.rect, before.2.rect);

                                if enabled && focused && !selected {
                                    let outline = |output: &FullOutput| {
                                        output
                                            .shapes
                                            .iter()
                                            .find_map(|shape| match &shape.shape {
                                                egui::Shape::Rect(rect)
                                                    if rect.rect == after.0.rect =>
                                                {
                                                    Some(rect.stroke)
                                                }
                                                _ => None,
                                            })
                                            .unwrap()
                                    };
                                    let focused_outline = outline(&after_output);
                                    assert!(focused_outline.width > 0.);
                                    assert!(focused_outline.color.a() > 0);
                                    assert_ne!(focused_outline, outline(&before_output));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn loading_and_error_cards_have_distinct_visible_cues() {
    for scale in [1., 2.] {
        for height in [52., 80., 140.] {
            let context = Context::default();
            context.set_pixels_per_point(scale);
            let paint = |content| {
                let fixture = CardFixture {
                    height,
                    enabled: true,
                    origin: egui::pos2(100., 10.),
                    selected: false,
                    content,
                };
                render(&context, fixture, vec![]);
                let ((_, image, _), output) = render(&context, fixture, vec![]);
                output
                    .shapes
                    .into_iter()
                    .filter(|shape| {
                        shape
                            .shape
                            .visual_bounding_rect()
                            .intersect(shape.clip_rect)
                            .intersects(image)
                    })
                    .collect::<Vec<_>>()
            };
            let empty = paint(Content::Empty);
            let loading = paint(Content::Loading);
            let error = paint(Content::Error);
            assert_ne!(loading, empty);
            assert_ne!(error, empty);
            assert_ne!(loading, error);
        }
    }
}

#[test]
fn captions_and_camera_badges_clip_to_their_regions_at_viewport_edges() {
    for scale in [1., 2.] {
        for height in [52., 80., 140.] {
            for x in [-35.25, 270.25] {
                let context = Context::default();
                context.set_pixels_per_point(scale);
                let draw = || {
                    frame(&context, 315., true, vec![], |ui| {
                        let layout = ContactSheetLayout::new(height - 12., height < 90.);
                        let card = layout.begin(
                            ui,
                            egui::pos2(x, 10.),
                            ui.id().with("clipped-card"),
                            false,
                            false,
                            true,
                        );
                        card.camera_badge(ui);
                        card.caption(ui, "01.ARW");
                        (card.image_rect, card.caption_rect, ui.clip_rect())
                    })
                };
                draw();
                let ((image, caption, clip), output) = draw();
                for (labels, region) in [
                    (&["Camera", "Camera preview"][..], image),
                    (&["01.ARW"][..], caption),
                ] {
                    let shape = output
                        .shapes
                        .iter()
                        .find(|shape| match &shape.shape {
                            egui::Shape::Text(text) => {
                                labels.contains(&text.galley.job.text.as_str())
                            }
                            _ => false,
                        })
                        .unwrap_or_else(|| {
                            panic!("missing {labels:?} at scale {scale}, height {height}, x {x}")
                        });
                    assert_eq!(shape.clip_rect, region.intersect(clip));
                }
            }
        }
    }
}

#[test]
fn card_responses_preserve_hover_mouse_keyboard_and_inherited_disabled_behavior() {
    let context = Context::default();
    let draw = |enabled, events| {
        render(
            &context,
            CardFixture {
                height: 80.,
                enabled,
                origin: egui::pos2(40., 10.),
                selected: false,
                content: Content::Loading,
            },
            events,
        )
    };
    let ((initial, _, following), _) = draw(true, vec![]);
    let point = initial.rect.center();
    let ((hover, _, _), _) = draw(true, vec![Event::PointerMoved(point)]);
    assert!(hover.hovered());
    draw(true, pointer(point, true));
    assert!(draw(true, pointer(point, false)).0.0.clicked());
    context.memory_mut(|memory| memory.request_focus(initial.id));
    assert!(draw(true, key(egui::Key::Space)).0.0.clicked());
    draw(true, key(egui::Key::Tab));
    assert_eq!(
        context.memory(|memory| memory.focused()),
        Some(following.id)
    );
    context.memory_mut(|memory| memory.request_focus(initial.id));
    assert!(!draw(false, key(egui::Key::Space)).0.0.clicked());
    draw(false, pointer(point, true));
    assert!(!draw(false, pointer(point, false)).0.0.clicked());
    assert_eq!(draw(false, vec![]).0.0.id, initial.id);
}

#[test]
fn card_availability_preserves_ids_captions_and_following_widgets() {
    let context = Context::default();
    let name = "京都_Summer_Scan_Reference_Roll01_Frame_04397.ARW";
    let draw = |enabled, selected, events| {
        frame(&context, 315., true, events, |ui| {
            let layout = ContactSheetLayout::new(40., true);
            let next_id = ui.next_auto_id();
            let available = ui.available_rect_before_wrap();
            let card = layout.begin(
                ui,
                egui::pos2(40., 10.),
                ui.id().with("card"),
                selected,
                false,
                enabled,
            );
            card.caption(ui, name);
            assert_eq!(ui.next_auto_id(), next_id);
            assert_eq!(ui.available_rect_before_wrap(), available);
            assert!(ui.is_enabled());
            (card.response, ui.button("Following"))
        })
    };
    let ((initial, following), _) = draw(true, true, vec![]);
    context.memory_mut(|memory| memory.request_focus(initial.id));
    let ((disabled, after), output) = draw(false, true, key(egui::Key::Space));
    assert_eq!(disabled.id, initial.id);
    assert_eq!(disabled.rect, initial.rect);
    assert_eq!(after.id, following.id);
    assert!(after.enabled());
    assert!(!disabled.enabled() && !disabled.clicked() && !disabled.has_focus());
    let node = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(name))
        .unwrap();
    assert_eq!(node.1.role(), egui::accesskit::Role::Button);
    assert_eq!(node.1.toggled(), Some(egui::accesskit::Toggled::True));
    assert!(node.1.is_disabled());
    let ((enabled, _), output) = draw(true, false, vec![]);
    assert_eq!(enabled.id, initial.id);
    let node = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(name))
        .unwrap();
    assert_eq!(node.1.toggled(), Some(egui::accesskit::Toggled::False));
    assert!(!node.1.is_disabled());
}

#[test]
fn layout_measurement_does_not_allocate_widgets_change_ids_or_consume_available_space() {
    let context = Context::default();
    frame(&context, 315., true, vec![], |ui| {
        let id = ui.next_auto_id();
        let available = ui.available_rect_before_wrap();
        for height in [52., 80., 140., 240.] {
            let layout = ContactSheetLayout::new(height - 12., height < 90.);
            assert!(layout.size().x >= 80.);
            assert_eq!(layout.pitch(), layout.size().x + 12.);
        }
        assert_eq!(ui.next_auto_id(), id);
        assert_eq!(ui.available_rect_before_wrap(), available);
    });
}

#[test]
fn active_background_preserves_geometry_selection_outline_and_filename_semantics() {
    for scale in [1., 2.] {
        for height in [52., 80., 140., 240.] {
            for enabled in [false, true] {
                let context = Context::default();
                context.set_pixels_per_point(scale);
                let draw = |active| {
                    frame(&context, 315., enabled, vec![], |ui| {
                        let layout = ContactSheetLayout::new(height - 12., height < 90.);
                        let next_id = ui.next_auto_id();
                        let available = ui.available_rect_before_wrap();
                        let card = layout.begin(
                            ui,
                            egui::pos2(240., 10.),
                            ui.id().with("active-card"),
                            true,
                            active,
                            true,
                        );
                        card.caption(ui, "full-source-filename.ARW");
                        assert_eq!(ui.next_auto_id(), next_id);
                        assert_eq!(ui.available_rect_before_wrap(), available);
                        (card.response, card.image_rect, card.caption_rect)
                    })
                };
                draw(false);
                let (baseline, before) = draw(false);
                let (active, output) = draw(true);
                assert_eq!(active.0.id, baseline.0.id);
                assert_eq!(active.0.rect, baseline.0.rect);
                assert_eq!(active.0.enabled(), baseline.0.enabled());
                assert_eq!(active.1, baseline.1);
                assert_eq!(active.2, baseline.2);
                let card_frame = |output: &FullOutput| {
                    output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if rect.rect == active.0.rect => {
                                Some((rect.clone(), shape.clip_rect))
                            }
                            _ => None,
                        })
                        .unwrap()
                };
                let (normal_frame, clip) = card_frame(&before);
                let (active_frame, active_clip) = card_frame(&output);
                assert!(active_frame.fill.r() > normal_frame.fill.r());
                assert_eq!(normal_frame.stroke, active_frame.stroke);
                assert_eq!(clip, active_clip);
                assert!(
                    active_frame.rect.contains_rect(active.2),
                    "caption shares the active background"
                );
                let image_well = |output: &FullOutput| {
                    output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if rect.rect == active.1 => Some(rect.fill),
                            _ => None,
                        })
                        .unwrap()
                };
                let normal_well = image_well(&before);
                let active_well = image_well(&output);
                assert_eq!(normal_well, normal_frame.fill);
                assert_eq!(
                    active_well, active_frame.fill,
                    "letterboxing follows the card background, without an extra black frame"
                );
                let (_, node) = output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(node_id, _)| *node_id == active.0.id.accesskit_id())
                    .unwrap();
                assert_eq!(node.label(), Some("full-source-filename.ARW"));
                assert_eq!(node.toggled(), Some(egui::accesskit::Toggled::True));
                assert_eq!(node.is_disabled(), !enabled);
                assert_eq!(node.description(), Some("Active photo"));
            }
        }
    }
}

#[test]
fn captions_are_single_line_and_middle_elided_without_losing_unicode_or_file_identity() {
    let context = Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
        for name in [
            "DSC04397.ARW",
            "Summer_Scan_Calibration_Reference_Roll01_Frame_04397.ARW",
            "京都_撮影_長い名称_Reference_Frame_04397.ARW",
        ] {
            let full = caption_name(ui, name, 1000., egui::Color32::WHITE);
            assert_eq!(full.job.text, name);
            for width in [54., 90., 140.] {
                let caption = caption_name(ui, name, width, egui::Color32::WHITE);
                assert_eq!(caption.rows.len(), 1);
                assert!(caption.size().x <= width);
                assert!(!caption.job.text.contains('\n'));
                assert!(!caption.job.text.contains('�'));
                if caption.job.text != name {
                    assert!(caption.job.text.contains('…'));
                    let (prefix, suffix) = caption.job.text.split_once('…').unwrap();
                    assert!(name.starts_with(prefix) && name.ends_with(suffix));
                    assert!(
                        caption.job.text.ends_with("97.ARW"),
                        "{width}pt: {}",
                        caption.job.text
                    );
                }
            }
        }
    });
    output.textures_delta.clear();
}
