use crate::editor::ui::test_support::{key_event as key, named_rect, pointer, wide_row};
use crate::editor::ui::{SectionHeader, section};
use egui::Response;

struct SectionFrame {
    numeric: Option<Response>,
    other: Option<Response>,
    output: egui::FullOutput,
}

fn section_frame(
    context: &egui::Context,
    value: &mut f64,
    events: Vec<egui::Event>,
) -> SectionFrame {
    context.enable_accesskit();
    let mut numeric = None;
    let mut other = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(315., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let changed = *value != 0.;
                let (_, action) = section(
                    ui,
                    "grade",
                    "Grade",
                    SectionHeader {
                        summary: &format!("{:+.2} EV", value),
                        changed,
                        reset: Some(changed),
                        tool: None,
                        default_open: Some(true),
                    },
                    |ui| {
                        numeric = Some(wide_row().show(ui, value).numeric);
                    },
                );
                if action.reset {
                    *value = 0.;
                }
                section(
                    ui,
                    "other",
                    "Other",
                    SectionHeader {
                        summary: "Independent section",
                        changed: false,
                        reset: None,
                        tool: None,
                        default_open: Some(true),
                    },
                    |ui| {
                        other = Some(ui.button("Other body"));
                    },
                );
            });
        },
    );
    output.textures_delta.clear();
    SectionFrame {
        numeric,
        other,
        output,
    }
}

#[test]
fn collapsed_sections_keep_adjustments_summaries_and_independent_resets() {
    let context = egui::Context::default();
    let mut value = 1.23456789;
    let first = section_frame(&context, &mut value, vec![]);
    let numeric_id = first.numeric.as_ref().unwrap().id;
    let other_id = first.other.as_ref().unwrap().id;
    let header = named_rect(&first.output, "Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    let collapsed = section_frame(&context, &mut value, vec![]);
    assert!(collapsed.numeric.is_none());
    assert_eq!(collapsed.other.as_ref().unwrap().id, other_id);
    assert_eq!(value, 1.23456789);
    assert!(
        collapsed
            .output
            .shapes
            .iter()
            .any(|shape| match &shape.shape {
                egui::Shape::Text(text) => text.galley.job.text.contains("+1.23 EV"),
                _ => false,
            })
    );
    let modified = |output: &egui::FullOutput| {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Grade"))
            .unwrap()
            .1
            .description()
            == Some("Modified")
    };
    assert!(modified(&collapsed.output));
    value = 2.;
    assert!(
        section_frame(&context, &mut value, vec![])
            .numeric
            .is_none()
    );
    let reset = named_rect(&collapsed.output, "Reset Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(reset, pressed));
    }
    let reset = section_frame(&context, &mut value, vec![]);
    assert_eq!(value, 0.);
    assert!(reset.numeric.is_none());
    assert!(!modified(&reset.output));
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    let reopened = section_frame(&context, &mut value, vec![]);
    assert_eq!(reopened.numeric.as_ref().unwrap().id, numeric_id);
    assert!(reopened.other.is_some());
}

#[test]
fn collapsing_a_pending_numeric_edit_cancels_without_committing_on_reopen() {
    let context = egui::Context::default();
    let mut value = 0.123456789;
    let first = section_frame(&context, &mut value, vec![]);
    let numeric = first.numeric.unwrap().rect.center();
    let header = named_rect(&first.output, "Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(numeric, pressed));
    }
    section_frame(&context, &mut value, vec![egui::Event::Text("5".into())]);
    assert_eq!(value, 0.123456789);
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    for _ in 0..3 {
        assert!(
            section_frame(&context, &mut value, vec![])
                .numeric
                .is_none()
        );
        assert_eq!(value, 0.123456789);
    }
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    let reopened = section_frame(&context, &mut value, vec![]);
    assert!(reopened.numeric.is_some());
    assert_eq!(value, 0.123456789);
    for pressed in [true, false] {
        section_frame(
            &context,
            &mut value,
            pointer(reopened.numeric.as_ref().unwrap().rect.center(), pressed),
        );
    }
    section_frame(
        &context,
        &mut value,
        vec![
            egui::Event::Text("-2".into()),
            key(egui::Key::Enter, egui::Modifiers::NONE),
        ],
    );
    section_frame(&context, &mut value, vec![]);
    assert_eq!(value, -2.);
}

#[test]
fn disclosure_supports_keyboard_activation_and_reports_expansion() {
    let context = egui::Context::default();
    let mut value = 0.;
    let first = section_frame(&context, &mut value, vec![]);
    let header = named_rect(&first.output, "Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    assert!(
        section_frame(&context, &mut value, vec![])
            .numeric
            .is_none()
    );
    section_frame(
        &context,
        &mut value,
        vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
    );
    section_frame(
        &context,
        &mut value,
        vec![key(egui::Key::Space, egui::Modifiers::NONE)],
    );
    let expanded = section_frame(&context, &mut value, vec![]);
    assert!(expanded.numeric.is_some());
    let node = &expanded
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Grade"))
        .unwrap()
        .1;
    assert_eq!(node.is_expanded(), Some(true));
}
