use super::super::{draggable_icon_row, icons::Icon};
use super::*;

fn draw(
    ctx: &Context,
    width: f32,
    available: bool,
    checked: &mut bool,
    selected: bool,
    events: Vec<Event>,
) -> ((Response, Response, Response), FullOutput) {
    frame(ctx, width, available, events, |ui| {
        ui.vertical(|ui| {
            let global = draggable_icon_row(ui, Icon::LinearGradient, "First", !selected, None).0;
            let (local, toggle) = draggable_icon_row(
                ui,
                Icon::RadialGradient,
                "A long radial adjustment name",
                selected,
                Some(&mut *checked),
            );
            (global, local, toggle.unwrap())
        })
        .inner
    })
}

#[test]
fn selectable_icon_rows_fill_width_expose_selection_and_keep_toggle_separate() {
    for width in [230., 315.] {
        let ctx = Context::default();
        let mut enabled = true;
        let ((global, local, toggle), output) = draw(&ctx, width, true, &mut enabled, true, vec![]);
        assert_eq!(global.rect.height(), super::super::CONTROL_HEIGHT);
        assert_eq!(local.rect.height(), global.rect.height());
        assert!(global.rect.left() >= 0. && global.rect.right() <= width);
        assert!(
            (local.rect.width() + super::super::CONTROL_HEIGHT - global.rect.width()).abs() < 0.01
        );
        assert!(local.rect.right() <= toggle.rect.left() + 0.01);
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        for (label, selected) in [("First", false), ("A long radial adjustment name", true)] {
            let node = nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .unwrap();
            assert_eq!(node.1.is_selected(), Some(selected));
        }
        let labels: Vec<_> = nodes.iter().filter_map(|(_, node)| node.label()).collect();
        assert!(labels.contains(&"Enable A long radial adjustment name"));
        for pressed in [true, false] {
            let ((_, local, _), _) = draw(
                &ctx,
                width,
                true,
                &mut enabled,
                true,
                pointer(toggle.rect.center(), pressed),
            );
            assert!(
                !local.clicked(),
                "the enable checkbox must not select its row"
            );
        }
        assert!(!enabled);
        let mut clicked = false;
        for pressed in [true, false] {
            let ((_, local, toggle), _) = draw(
                &ctx,
                width,
                true,
                &mut enabled,
                true,
                pointer(local.rect.center(), pressed),
            );
            clicked |= local.clicked();
            assert!(!toggle.changed());
        }
        assert!(clicked);
        assert!(!enabled);
    }
}

#[test]
fn draggable_rows_capture_movement_without_toggling_and_keep_keyboard_and_disable_semantics() {
    for available in [true, false] {
        let ctx = Context::default();
        let mut checked = true;
        let mut draw = |events| {
            frame(&ctx, 270., available, events, |ui| {
                super::super::draggable_icon_row(
                    ui,
                    Icon::LinearGradient,
                    "Local",
                    true,
                    Some(&mut checked),
                )
            })
        };
        let ((row, _), _) = draw(vec![]);
        let start = row.rect.center();
        draw(pointer(start, true));
        let ((moved, toggle), _) = draw(vec![Event::PointerMoved(start + egui::vec2(25., 0.))]);
        assert_eq!(moved.drag_started(), available);
        assert!(!moved.clicked() && !toggle.unwrap().changed());
        draw(pointer(start + egui::vec2(25., 0.), false));
        ctx.memory_mut(|memory| memory.request_focus(row.id));
        let ((row, _), _) = draw(key(egui::Key::Space));
        assert_eq!(row.clicked(), available);
        assert!(checked);
    }
}

#[test]
fn selectable_icon_rows_keep_keyboard_activation_and_inherited_disable() {
    let ctx = Context::default();
    let mut enabled = true;
    let ((_, local, _), _) = draw(&ctx, 270., true, &mut enabled, false, vec![]);
    ctx.memory_mut(|memory| memory.request_focus(local.id));
    let ((_, local, _), _) = draw(&ctx, 270., true, &mut enabled, false, key(egui::Key::Space));
    assert!(local.clicked());
    let ((_, local, toggle), _) = draw(&ctx, 270., false, &mut enabled, false, vec![]);
    assert!(!local.enabled() && !toggle.enabled());
    for point in [local.rect.center(), toggle.rect.center()] {
        for pressed in [true, false] {
            let ((_, row, toggle), _) = draw(
                &ctx,
                270.,
                false,
                &mut enabled,
                false,
                pointer(point, pressed),
            );
            assert!(!row.clicked() && !toggle.changed());
        }
    }
    assert!(enabled);
}
