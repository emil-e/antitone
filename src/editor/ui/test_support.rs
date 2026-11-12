use super::ParameterRow;
use egui::{Context, Event, FullOutput, Modifiers, Pos2, Rect, Ui};

pub(in crate::editor) fn frame<R>(
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

pub(in crate::editor) fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Option-click on the value name: the single-action reset gesture.
pub(in crate::editor) fn alt_click(position: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![
        egui::Event::ModifiersChanged(egui::Modifiers::ALT),
        egui::Event::PointerMoved(position),
    ];
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::ALT,
        });
    }
    events
}

pub(in crate::editor) fn named_rect(output: &egui::FullOutput, name: &str) -> egui::Rect {
    let node = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(name))
        .unwrap()
        .1;
    let rect = node.bounds().unwrap();
    egui::Rect::from_min_max(
        egui::pos2(rect.x0 as f32, rect.y0 as f32),
        egui::pos2(rect.x1 as f32, rect.y1 as f32),
    )
}

pub(in crate::editor) fn wide_row() -> ParameterRow<'static> {
    ParameterRow::new("wide", "Exposure", -16.0..=16.0)
        .slider_range(-3.0..=3.0)
        .default_value(0.)
        .step(0.05)
        .speed(0.01)
        .unit("EV")
}
