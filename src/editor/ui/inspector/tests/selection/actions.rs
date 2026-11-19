use super::*;
use crate::editor::ui::ToolButton;

#[derive(Default)]
struct State {
    power: [bool; 2],
    editing: bool,
}

struct Rows {
    global: Response,
    global_power: Response,
    mask: Response,
    power: Response,
    edit: Response,
}

fn render(
    context: &Context,
    width: f32,
    available: bool,
    state: &mut State,
    events: Vec<Event>,
) -> (Rows, FullOutput) {
    let (rows, output) = frame(context, width, available, events, |ui| {
        ui.vertical(|ui| {
            let (global, global_power, no_edit) = draggable_icon_row(
                ui,
                Icon::GlobalAdjustmentLayer,
                "Whole photo",
                false,
                Some(&mut state.power[0]),
                None,
            );
            assert!(no_edit.is_none());
            let (mask, power, edit) = draggable_icon_row(
                ui,
                Icon::LinearGradient,
                "Sky",
                true,
                Some(&mut state.power[1]),
                Some(ToolButton::tool(
                    Icon::Edit,
                    "Edit Sky",
                    "Edit mask",
                    state.editing,
                )),
            );
            Rows {
                global,
                global_power: global_power.unwrap(),
                mask,
                power: power.unwrap(),
                edit: edit.unwrap(),
            }
        })
        .inner
    });
    if rows.edit.clicked() {
        state.editing = !state.editing;
    }
    (rows, output)
}

#[test]
fn row_actions_align_without_shortening_the_selection_background() {
    for width in [230., 270., 315.] {
        for scale in [1., 2.] {
            let context = Context::default();
            context.set_pixels_per_point(scale);
            let mut state = State {
                power: [true; 2],
                editing: true,
            };
            render(&context, width, true, &mut state, vec![]);
            let (rows, output) = render(&context, width, true, &mut state, vec![]);
            assert_eq!(rows.global.rect.x_range(), rows.mask.rect.x_range());
            assert_eq!(rows.global_power.rect.x_range(), rows.power.rect.x_range());
            assert!(rows.mask.rect.right() <= rows.edit.rect.left());
            assert!(rows.edit.rect.right() <= rows.power.rect.left());
            let full_row = rows.mask.rect.union(rows.power.rect);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.rect == full_row && rect.fill != egui::Color32::TRANSPARENT
            )), "selection background includes both action columns");
            let nodes = &output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes;
            let edit = nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Edit Sky"))
                .unwrap();
            assert_eq!(edit.1.is_selected(), Some(true));
            assert!(
                !nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Edit Whole photo"))
            );
        }
    }
}

#[test]
fn edit_and_power_capture_pointer_and_keyboard_without_selecting_or_dragging() {
    for available in [false, true] {
        let context = Context::default();
        let mut state = State {
            power: [true; 2],
            editing: false,
        };
        render(&context, 270., available, &mut state, vec![]);
        let (rows, _) = render(&context, 270., available, &mut state, vec![]);
        for pressed in [true, false] {
            let (clicked, _) = render(
                &context,
                270.,
                available,
                &mut state,
                pointer(rows.edit.rect.center(), pressed),
            );
            assert!(!clicked.mask.clicked() && !clicked.mask.drag_started());
            assert!(!clicked.power.changed());
        }
        assert_eq!(state.editing, available);
        for pressed in [true, false] {
            let (clicked, _) = render(
                &context,
                270.,
                available,
                &mut state,
                pointer(rows.power.rect.center(), pressed),
            );
            assert!(
                !clicked.mask.clicked() && !clicked.mask.drag_started() && !clicked.edit.clicked()
            );
        }
        assert_eq!(state.power, [true, !available]);
        context.memory_mut(|memory| memory.request_focus(rows.edit.id));
        render(&context, 270., available, &mut state, key(egui::Key::Space));
        assert!(
            !state.editing,
            "a second activation exits editing; unavailable actions do nothing"
        );
        context.memory_mut(|memory| memory.request_focus(rows.power.id));
        render(&context, 270., available, &mut state, key(egui::Key::Space));
        assert_eq!(state.power, [true; 2]);
    }
}

#[test]
fn bypass_changes_state_and_tone_but_uses_the_same_power_artwork() {
    let context = Context::default();
    let mut state = State {
        power: [true; 2],
        editing: false,
    };
    render(&context, 270., true, &mut state, vec![]);
    let (on, on_output) = render(&context, 270., true, &mut state, vec![]);
    state.power[1] = false;
    let (off, off_output) = render(&context, 270., true, &mut state, vec![]);
    let artwork = |output: &FullOutput, rect: egui::Rect| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Path(path)
                    if path.points.iter().all(|point| rect.contains(*point)) =>
                {
                    Some(path.points.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let paths = artwork(&on_output, on.power.rect);
    assert!(!paths.is_empty());
    assert_eq!(
        paths,
        artwork(&off_output, off.power.rect),
        "no alternate or struck-through glyph"
    );
    assert!(
        off.power.enabled() && off.edit.enabled() && off.mask.enabled(),
        "bypass is not UI disablement"
    );
    for (output, checked) in [(&on_output, true), (&off_output, false)] {
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let power = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Enable Sky"))
            .unwrap();
        assert_eq!(
            power.1.toggled(),
            Some(if checked {
                egui::accesskit::Toggled::True
            } else {
                egui::accesskit::Toggled::False
            })
        );
    }
    let text_color = |output: &FullOutput| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Sky" => {
                    text.override_text_color
                }
                _ => None,
            })
            .unwrap()
    };
    assert_ne!(
        text_color(&on_output),
        text_color(&off_output),
        "bypassed names are visibly muted"
    );
}
