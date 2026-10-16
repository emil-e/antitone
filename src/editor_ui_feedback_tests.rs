use super::*;
use crate::editor::presentation::tests::{key, pointer};
use egui::{Event, FullOutput, Pos2, Rect};

fn frame(
    context: &Context,
    width: f32,
    severity: Severity,
    events: Vec<Event>,
    details: &str,
    escape_available: bool,
) -> (bool, FullOutput) {
    context.enable_accesskit();
    let mut acknowledged = false;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.))),
            events,
            ..Default::default()
        },
        |ui| {
            acknowledged = FeedbackDialog {
                severity,
                title: "Operation could not complete",
                explanation: "Your accepted settings are unchanged. Review the details before trying again.",
                details,
            }
            .show(ui.ctx(), Id::new("test-feedback"), escape_available);
        },
    );
    output.textures_delta.clear();
    (acknowledged, output)
}

fn text(output: &FullOutput, label: &str) -> Option<Rect> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.job.text == label => {
            Some(Rect::from_min_size(text.pos, text.galley.size()))
        }
        _ => None,
    })
}

#[test]
fn severity_is_written_accessible_and_visually_distinct_with_safe_acknowledgement() {
    for (severity, label) in [
        (Severity::Information, "Information"),
        (Severity::Warning, "Warning"),
        (Severity::Error, "Error"),
    ] {
        let context = Context::default();
        frame(&context, 800., severity, vec![], "technical reason", true);
        let (_, output) = frame(&context, 800., severity, vec![], "technical reason", true);
        assert!(text(&output, label).is_some());
        assert!(text(&output, "technical reason").is_none());
        assert!(text(&output, "Technical details").is_none());
        assert!(output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => {
                text.galley.job.text == "Details"
                    && text.galley.job.sections[0].format.color
                        == context.global_style().visuals.text_color()
            }
            _ => false,
        }));
        assert!(!output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::LineSegment { points, .. }
            if (points[0].y - points[1].y).abs() < 0.1 && (points[0].x - points[1].x).abs() > 200.)
        ));
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let disclosure = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Details"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        assert!(disclosure.width() < 100.);
        assert_eq!(disclosure.height(), 22.);
        assert!(nodes.iter().any(|(_, node)| node.value() == Some(label)));
        assert!(
            nodes
                .iter()
                .any(|(_, node)| node.label() == Some("OK") && !node.is_disabled())
        );
        let expected = match severity {
            Severity::Information => context.global_style().visuals.selection.stroke.color,
            Severity::Warning => context.global_style().visuals.warn_fg_color,
            Severity::Error => context.global_style().visuals.error_fg_color,
        };
        assert!(output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => {
                text.galley.job.text == label
                    && text.galley.job.sections[0].format.color == expected
            }
            _ => false,
        }));
        for pressed in [true, false] {
            assert!(
                !frame(
                    &context,
                    800.,
                    severity,
                    pointer(egui::pos2(20., 20.), pressed),
                    "technical reason",
                    true
                )
                .0
            );
        }
        assert!(
            !frame(
                &context,
                800.,
                severity,
                key(egui::Key::Escape),
                "technical reason",
                false
            )
            .0
        );
        assert!(
            frame(
                &context,
                800.,
                severity,
                key(egui::Key::Escape),
                "technical reason",
                true
            )
            .0
        );
        assert!(
            frame(
                &context,
                800.,
                severity,
                key(egui::Key::Enter),
                "technical reason",
                true
            )
            .0
        );
    }
}

#[test]
fn technical_disclosure_owns_focus_is_bounded_and_does_not_join_inspector_preferences() {
    for width in [800., 320., 260.] {
        let context = Context::default();
        let preferences = std::collections::BTreeMap::from([("photo-contrast".into(), true)]);
        hierarchy::restore_expansion(&context, preferences.clone());
        let details =
            "Very long technical explanation and /a/long/path/to/a/source-file.ARW\n".repeat(100);
        frame(&context, width, Severity::Error, vec![], &details, true);
        let (_, output) = frame(&context, width, Severity::Error, vec![], &details, true);
        let point = text(&output, "Details").unwrap().center();
        for pressed in [true, false] {
            assert!(
                !frame(
                    &context,
                    width,
                    Severity::Error,
                    pointer(point, pressed),
                    &details,
                    true
                )
                .0
            );
        }
        let (_, output) = frame(&context, width, Severity::Error, vec![], &details, true);
        assert!(text(&output, &details).is_some());
        let area = context.memory(|memory| memory.area_rect(Id::new("test-feedback")).unwrap());
        assert!(area.height() < 580., "{area:?}");
        assert!(area.min.x >= 7. && area.max.x <= width - 7.);
        assert!(area.contains_rect(text(&output, "OK").unwrap()));
        let focused = context.memory(|memory| memory.focused().unwrap());
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(
            !nodes
                .iter()
                .any(|(id, node)| id.0 == focused.value() && node.label() == Some("OK"))
        );
        assert_eq!(
            context.read_response(focused).unwrap().layer_id.id,
            Id::new("test-feedback")
        );
        assert!(
            !frame(
                &context,
                width,
                Severity::Error,
                key(egui::Key::Enter),
                &details,
                true
            )
            .0
        );
        assert_eq!(hierarchy::collect_expansion(&context), preferences);
    }
}
