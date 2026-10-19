use super::*;
use crate::editor::presentation::tests::{key, pointer};
use egui::{Event, FullOutput, Pos2, Rect};

fn frame(
    context: &Context,
    width: f32,
    measured: bool,
    time: f64,
    events: Vec<Event>,
) -> (bool, FullOutput) {
    context.enable_accesskit();
    let mut clicked = false;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| {
            clicked = ui.button("Underlying action").clicked();
            assert!(
                !ProgressContent {
                    title: "Importing RAWs…",
                    progress: if measured {
                        Progress::Measured {
                            fraction: 0.25,
                            label: "3 of 12",
                        }
                    } else {
                        Progress::Indeterminate
                    },
                    cancel: None,
                }
                .modal(ui.ctx(), Id::new("test-working"))
            );
        },
    );
    output.textures_delta.clear();
    (clicked, output)
}

#[test]
fn working_modals_keep_the_operation_visible_and_accessible_at_narrow_widths() {
    for width in [1100., 800., 320., 260.] {
        let context = Context::default();
        frame(&context, width, false, 1., vec![]);
        frame(&context, width, false, 1., vec![]);
        let (_, output) = frame(&context, width, false, 1., vec![]);
        let area = context.memory(|memory| memory.area_rect(Id::new("test-working")).unwrap());
        assert!(
            Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.)).contains_rect(area),
            "{area:?}"
        );
        let texts = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text != "Underlying action" => {
                    Some(text)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            texts
                .iter()
                .any(|text| text.galley.job.text == "Importing RAWs…")
        );
        for text in texts {
            assert!(area.contains_rect(Rect::from_min_size(text.pos, text.galley.size())));
        }
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let progress = nodes
            .iter()
            .find(|(_, node)| node.role() == egui::accesskit::Role::ProgressIndicator)
            .unwrap();
        assert!(progress.1.numeric_value().is_none());
        assert_eq!(progress.1.label(), Some("Importing RAWs…"));
        assert!(
            !nodes
                .iter()
                .any(|(_, node)| matches!(node.label(), Some("Cancel" | "OK")))
        );
    }
}

#[test]
fn modal_blocks_background_and_cannot_be_dismissed_by_backdrop_escape_or_enter() {
    let context = Context::default();
    frame(&context, 800., false, 1., vec![]);
    let (_, output) = frame(&context, 800., false, 1., vec![]);
    let point = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Underlying action" => {
                Some(text.pos + text.galley.size() / 2.)
            }
            _ => None,
        })
        .unwrap();
    frame(&context, 800., false, 1., pointer(point, true));
    assert!(!frame(&context, 800., false, 1., pointer(point, false)).0);
    frame(&context, 800., false, 1., key(egui::Key::Escape));
    frame(&context, 800., false, 1., key(egui::Key::Enter));
    let (_, output) = frame(&context, 800., false, 1., vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Importing RAWs…")));
}

#[test]
fn indeterminate_bar_moves_without_exposing_a_percentage() {
    let context = Context::default();
    frame(&context, 800., false, 0., vec![]);
    frame(&context, 800., false, 1., vec![]);
    let (_, first) = frame(&context, 800., false, 1., vec![]);
    let (_, second) = frame(&context, 800., false, 1.4, vec![]);
    let bar_shapes = |output: &FullOutput| {
        let bounds = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::ProgressIndicator
                    && node.label() == Some("Importing RAWs…")
            })
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let bar = Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        );
        let shapes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if shape.clip_rect == bar => Some(rect.rect),
                _ => None,
            })
            .collect();
        assert!(
            !shapes.is_empty(),
            "progress artwork must be present in its named region"
        );
        shapes
    };
    assert_ne!(bar_shapes(&first), bar_shapes(&second));
    assert!(
        second
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::ProgressIndicator)
            .all(|(_, node)| node.numeric_value().is_none())
    );
}

#[test]
fn measured_progress_uses_supplied_file_counts() {
    let context = Context::default();
    frame(&context, 800., true, 1., vec![]);
    let (_, output) = frame(&context, 800., true, 1., vec![]);
    assert!(
        output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("3 of 12") && node.numeric_value() == Some(25.))
    );
}

#[test]
fn viewer_activity_stays_inside_the_canvas_with_an_accessible_operation_name() {
    let context = Context::default();
    context.enable_accesskit();
    let canvas = Rect::from_min_size(egui::pos2(32., 60.), egui::vec2(500., 350.));
    let mut output = context.run_ui(Default::default(), |ui| {
        let response = ProgressContent {
            title: "Loading photo…",
            progress: Progress::Indeterminate,
            cancel: None,
        }
        .viewer(ui, canvas);
        assert_eq!(response.rect.center(), canvas.center());
        assert!(response.rect.is_positive());
        assert!(canvas.contains_rect(response.rect));
    });
    output.textures_delta.clear();
    assert!(
        output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Loading photo…"))
    );
}

#[test]
fn cancellation_requires_the_supplied_action_in_the_compact_row() {
    let context = Context::default();
    let frame = |events| {
        let mut cancelled = false;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
                events,
                ..Default::default()
            },
            |ui| {
                cancelled = ProgressContent {
                    title: "Analyzing roll…",
                    progress: Progress::Measured {
                        fraction: 0.25,
                        label: "3 of 12",
                    },
                    cancel: Some("Cancel analysis"),
                }
                .compact(ui);
            },
        );
        output.textures_delta.clear();
        (cancelled, output)
    };
    assert!(!frame(key(egui::Key::Escape)).0);
    let (_, output) = frame(vec![]);
    let point = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Cancel analysis" => {
                Some(text.pos + text.galley.size() / 2.)
            }
            _ => None,
        })
        .unwrap();
    assert!(!frame(pointer(point, true)).0);
    assert!(frame(pointer(point, false)).0);
}
