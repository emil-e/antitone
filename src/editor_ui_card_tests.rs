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

// Direct accepted card artwork is an equivalence oracle, not another production path.
fn direct(
    ui: &mut Ui,
    available_height: f32,
    compact: bool,
    origin: Pos2,
    id: Id,
    selected: bool,
    content: Content,
) -> (Response, Rect) {
    let padding = if compact { 3. } else { 5. };
    let caption_height = if compact { 14. } else { 18. };
    let caption_gap = if compact { 2. } else { 4. };
    let image_height =
        (available_height - 6. - 2. * padding - caption_height - caption_gap).max(8.);
    let image_width = (image_height * 1.5 + 2. * padding).max(80.);
    let card_height = image_height + 2. * padding + caption_height + caption_gap;
    let cell = Rect::from_min_size(origin, egui::vec2(image_width, card_height));
    let response = ui.interact(cell, id, egui::Sense::click());
    let hovered = response.hovered();
    let fill = Color32::from_gray(if selected {
        30
    } else if hovered {
        28
    } else {
        24
    });
    let stroke = if selected || response.has_focus() {
        egui::Stroke::new(1.5, ui.visuals().selection.stroke.color)
    } else {
        egui::Stroke::new(1., Color32::from_gray(if hovered { 65 } else { 42 }))
    };
    let text_color = Color32::from_gray(if selected {
        225
    } else if hovered {
        195
    } else {
        165
    });
    ui.painter()
        .rect(cell, 4., fill, stroke, egui::StrokeKind::Inside);
    let image_rect = Rect::from_min_size(
        cell.min + egui::vec2(padding, padding),
        egui::vec2(image_width - 2. * padding, image_height),
    );
    ui.painter()
        .rect_filled(image_rect, 0., Color32::from_gray(14));
    match content {
        Content::Empty => (),
        Content::Loading | Content::Error => {
            ui.painter().text(
                image_rect.center(),
                egui::Align2::CENTER_CENTER,
                if matches!(content, Content::Loading) {
                    "…"
                } else {
                    "!"
                },
                egui::FontId::proportional((image_rect.height() / 1.2).min(16.)),
                text_color,
            );
        }
        Content::Camera => {
            camera_image(ui, image_rect);
            let painter = ui
                .painter()
                .with_clip_rect(image_rect.intersect(ui.clip_rect()));
            let font = egui::FontId::proportional(((image_rect.height() - 2.) / 1.2).min(10.));
            let galley = painter.layout_no_wrap(
                if image_rect.height() < 24. {
                    "Camera"
                } else {
                    "Camera preview"
                }
                .into(),
                font,
                Color32::WHITE,
            );
            let size = (galley.size() + egui::vec2(4., 2.)).min(image_rect.size());
            let label =
                Rect::from_min_size(image_rect.left_bottom() - egui::vec2(0., size.y), size);
            painter.rect_filled(label, 2., Color32::from_black_alpha(190));
            painter.galley(label.center() - galley.size() * 0.5, galley, Color32::WHITE);
        }
    }
    let caption = Rect::from_min_size(
        egui::pos2(image_rect.left(), image_rect.bottom() + caption_gap),
        egui::vec2(image_rect.width(), caption_height),
    );
    let galley =
        ui.painter()
            .layout_no_wrap("01.ARW".into(), egui::FontId::proportional(11.), text_color);
    ui.painter()
        .with_clip_rect(caption.intersect(ui.clip_rect()))
        .galley(caption.center() - galley.size() * 0.5, galley, text_color);
    (response, image_rect)
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
    shared: bool,
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
        let (response, image_rect) = if shared {
            let card = layout.begin(ui, origin, id, selected);
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
            (card.response, card.image_rect)
        } else {
            direct(
                ui,
                height - 12.,
                height < 90.,
                origin,
                id,
                selected,
                content,
            )
        };
        (response, image_rect, ui.button("Following"))
    })
}

#[test]
fn card_geometry_paint_ids_and_clips_match_direct_compact_normal_and_focused_states() {
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
                                let (baseline, _) = render(&context, false, fixture, vec![]);
                                if focused {
                                    context
                                        .memory_mut(|memory| memory.request_focus(baseline.0.id));
                                }
                                let (baseline, before) = render(&context, false, fixture, vec![]);
                                let (shared, after) = render(&context, true, fixture, vec![]);
                                assert_eq!(shared.0.id, baseline.0.id);
                                assert_eq!(shared.0.rect, baseline.0.rect);
                                assert_eq!(shared.0.enabled(), baseline.0.enabled());
                                assert_eq!(shared.0.has_focus(), baseline.0.has_focus());
                                assert_eq!(shared.1, baseline.1);
                                assert_eq!(shared.2.id, baseline.2.id);
                                assert_eq!(shared.2.rect, baseline.2.rect);
                                assert_eq!(after.shapes, before.shapes);
                                assert_eq!(
                                    after.platform_output.accesskit_update,
                                    before.platform_output.accesskit_update
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn card_responses_preserve_hover_mouse_keyboard_and_inherited_disabled_behavior() {
    for shared in [false, true] {
        let context = Context::default();
        let draw = |enabled, events| {
            render(
                &context,
                shared,
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
