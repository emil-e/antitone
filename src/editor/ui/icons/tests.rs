use super::*;
use crate::editor::ui::test_support::{frame, key_event as key, named_rect, pointer};
use egui::Context;

#[test]
fn rotation_arrowheads_extend_along_endpoint_tangents_and_are_filled() {
    let [arc, start, end] = rotation_cursor_artwork();
    assert!(arc[12].length() < 1e-5);
    let context = egui::Context::default();
    let image = rotation_cursor(&context, egui::Id::new("rotation-cursor"), 0.);
    for (endpoint, neighbour, head) in [(arc[0], arc[1], start), (arc[24], arc[23], end)] {
        let base = (head[0] + head[2]) * 0.5;
        let forward = head[1] - base;
        let radial = endpoint - egui::vec2(0., 8.);
        assert!((base - endpoint).length() < 1e-5);
        assert!(forward.dot(radial).abs() < 1e-5);
        assert!(forward.dot(endpoint - neighbour) > 0.);
        assert!((forward.length() - 3.2).abs() < 1e-5);
        let inside = Vec2::splat(15.5) + (head[0] + head[1] + head[2]) / 3.;
        let index = (inside.y.floor() as usize * 31 + inside.x.floor() as usize) * 4;
        assert_eq!(&image.rgba[index..index + 4], &[255; 4]);
    }
}

#[test]
fn rotation_cursor_bitmaps_keep_the_hotspot_contrast_and_bounded_storage() {
    let context = Context::default();
    let id = egui::Id::new("rotation-cursor");
    for step in 0..32 {
        let angle = step as f32 * std::f32::consts::TAU / 32.;
        let image = super::rotation_cursor(&context, id, angle);
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
        let reused = super::rotation_cursor(&context, id, angle);
        assert!(std::sync::Arc::ptr_eq(&image.rgba, &reused.rgba));
    }
    let original = super::rotation_cursor(&context, id, 0.);
    let turned = super::rotation_cursor(&context, id, std::f32::consts::FRAC_PI_2);
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
        super::rotation_cursor(ui.ctx(), id, 0.);
        assert_eq!(ui.next_auto_id(), before);
    });
}

#[test]
fn outline_icon_buttons_have_named_hit_targets_and_keyboard_actions() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut selected = false;
    let mut render = |events| {
        let mut response = None;
        let mut output = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                response = Some(super::toggle(
                    ui,
                    Icon::Guides,
                    &mut selected,
                    "Guides",
                    "Show composition guides",
                ));
            },
        );
        output.textures_delta.clear();
        (response.unwrap(), output)
    };
    let (response, output) = render(vec![]);
    assert_eq!(response.rect.size(), egui::Vec2::splat(28.));
    assert_eq!(named_rect(&output, "Guides"), response.rect);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Path(_)))
    );
    context.memory_mut(|memory| memory.request_focus(response.id));
    render(vec![key(egui::Key::Space, egui::Modifiers::NONE)]);
    drop(render);
    assert!(selected);
}

#[test]
fn segmented_icons_keep_one_selection_within_a_shared_frame() {
    let context = Context::default();
    let mut selected = 0;
    let mut render = |enabled, events| {
        frame(&context, 200., enabled, events, |ui| {
            let selector = IconSegments {
                choices: &[
                    (0, Icon::Vectorscope, "First"),
                    (1, Icon::Waveform, "Second"),
                ],
                height: 24.,
            };
            let width = selector.width(ui);
            let rect = selector.show(ui, &mut selected).rect;
            assert_eq!(rect.size(), egui::vec2(width, selector.height));
            (rect, ui.visuals().selection.bg_fill)
        })
    };
    let filled = |output: &egui::FullOutput, color, label| {
        let center = named_rect(output, label).center();
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(rect)
                if rect.fill == color && rect.rect.contains(center))
        })
    };

    let ((group, color), initial) = render(true, vec![]);
    let first = named_rect(&initial, "First");
    let second = named_rect(&initial, "Second");
    assert!(group.contains_rect(first.union(second)));
    assert_eq!(first.right(), second.left());
    assert!(filled(&initial, color, "First"));
    assert!(!filled(&initial, color, "Second"));

    for _ in 0..2 {
        render(true, pointer(second.center(), true));
        render(true, pointer(second.center(), false));
        let (_, output) = render(true, vec![egui::Event::PointerGone]);
        assert!(!filled(&output, color, "First"));
        assert!(filled(&output, color, "Second"));
    }

    render(false, pointer(first.center(), true));
    render(false, pointer(first.center(), false));
    drop(render);
    assert_eq!(selected, 1, "a disabled choice cannot change the selection");
}

#[test]
fn disabled_outline_icons_are_dimmed_and_cannot_activate() {
    let context = egui::Context::default();
    let render = |enabled, events| {
        let mut response = None;
        let mut output = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                response = Some(super::button(
                    ui,
                    Icon::Reset,
                    enabled,
                    "Reset",
                    "Reset value",
                ));
            },
        );
        output.textures_delta.clear();
        let stroke = output
            .shapes
            .into_iter()
            .find_map(|shape| {
                if let egui::Shape::Path(path) = shape.shape {
                    Some(path.stroke.color)
                } else {
                    None
                }
            })
            .unwrap();
        (response.unwrap(), stroke)
    };
    let (enabled, normal) = render(true, vec![]);
    let (disabled, dimmed) = render(false, vec![]);
    assert_eq!(enabled.id, disabled.id);
    assert_eq!(enabled.rect, disabled.rect);
    assert_ne!(normal, dimmed);
    assert!(!disabled.enabled());
    for pressed in [true, false] {
        assert!(
            !render(false, pointer(disabled.rect.center(), pressed))
                .0
                .clicked()
        );
    }
}
