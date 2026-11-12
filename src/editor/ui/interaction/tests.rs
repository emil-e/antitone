use super::*;
use crate::editor::ui::test_support::{pointer, wide_row};

#[test]
fn discrete_widget_responses_preserve_availability_without_retaining_an_edit() {
    for enabled in [true, false] {
        let context = egui::Context::default();
        let mut checked = false;
        let mut draw = |events| {
            let mut result = None;
            let mut output = context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let widget =
                        ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, "Enable"));
                    result = Some((ControlResponse::discrete(&widget), widget));
                },
            );
            output.textures_delta.clear();
            result.unwrap()
        };
        let (idle, widget) = draw(vec![]);
        assert_eq!(idle.id, widget.id);
        assert_eq!(idle.enabled, enabled);
        assert!(!idle.changed && !idle.active && !idle.cancelled);
        draw(pointer(widget.rect.center(), true));
        let (clicked, _) = draw(pointer(widget.rect.center(), false));
        assert_eq!(clicked.changed, enabled);
        assert!(!clicked.active && !clicked.cancelled);
        assert_eq!(checked, enabled, "enabled fixture is a positive control");
    }
}

#[test]
fn value_activity_is_frame_local_and_ends_when_controls_are_hidden_or_disabled() {
    let context = egui::Context::default();
    let mut value = 0.;
    let mut render = |visible: bool, enabled: bool, events| {
        let mut slider = None;
        let mut active = false;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(315., 200.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                if visible {
                    slider = Some(
                        ui.add_enabled_ui(enabled, |ui| wide_row().show(ui, &mut value).slider)
                            .inner,
                    );
                }
                active = values_are_being_adjusted(ui.ctx());
            },
        );
        output.textures_delta.clear();
        (slider, active)
    };
    let (slider, active) = render(true, true, vec![]);
    assert!(!active);
    let point = slider.unwrap().rect.center();
    assert!(render(true, true, pointer(point, true)).1);
    assert!(render(true, true, vec![]).1);
    assert!(!render(false, true, vec![]).1);
    assert!(!render(true, false, vec![]).1);
    assert!(!render(true, true, pointer(point, false)).1);
}
