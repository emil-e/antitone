use super::*;
use crate::editor::presentation::tests::{key, pointer};
use egui::{Event, FullOutput, Pos2, Rect};

fn frame(
    context: &Context,
    width: f32,
    events: Vec<Event>,
) -> (Option<DialogChoice>, bool, FullOutput) {
    context.enable_accesskit();
    let mut choice = None;
    let mut underlying = false;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.))),
            events,
            ..Default::default()
        },
        |ui| {
            underlying = ui.button("Underlying action").clicked();
            choice = DecisionDialog::new(
                "Save changes to this project?",
                "Opening another project without saving will discard unsaved edits in this project.",
                "Save changes",
            )
            .detail("/projects/照片-very-long-project-with-no-spaces-that-must-wrap.antitone.json")
            .alternative("Discard changes")
            .show(ui.ctx(), Id::new("test-decision"), true);
        },
    );
    output.textures_delta.clear();
    (choice, underlying, output)
}

fn text(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"))
}

#[test]
fn decision_layout_is_centred_padded_and_keeps_actions_readable_at_narrow_widths() {
    for width in [1100., 800., 320., 260.] {
        let context = Context::default();
        frame(&context, width, vec![]);
        frame(&context, width, vec![]);
        let (_, _, output) = frame(&context, width, vec![]);
        let area = context.memory(|memory| memory.area_rect(Id::new("test-decision")).unwrap());
        assert!((area.center().x - width / 2.).abs() < 1., "{area:?}");
        assert!((area.center().y - 300.).abs() < 1., "{area:?}");
        assert!(area.min.x >= 7. && area.max.x <= width - 7., "{area:?}");
        for label in [
            "Save changes to this project?",
            "Cancel",
            "Discard changes",
            "Save changes",
            "/projects/照片-very-long-project-with-no-spaces-that-must-wrap.antitone.json",
        ] {
            let rect = text(&output, label);
            assert!(
                area.contains_rect(rect),
                "{label}: {rect:?} outside {area:?}"
            );
            assert!(rect.min.x >= area.min.x + 19. && rect.max.x <= area.max.x - 19.);
        }
        let cancel = text(&output, "Cancel");
        let alternative = text(&output, "Discard changes");
        let confirm = text(&output, "Save changes");
        let detail = text(
            &output,
            "/projects/照片-very-long-project-with-no-spaces-that-must-wrap.antitone.json",
        );
        assert!(cancel.min.y - detail.max.y >= 16.);
        if width == 260. {
            assert!(cancel.max.y < alternative.min.y && alternative.max.y < confirm.min.y);
        } else {
            assert!((cancel.min.y - confirm.min.y).abs() < 1.);
            assert!(cancel.max.x < alternative.min.x && alternative.max.x < confirm.min.x);
        }
        assert!(confirm.max.x >= area.max.x - 30.);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(
            !nodes
                .iter()
                .any(|(_, node)| matches!(node.value(), Some("Warning" | "Information" | "Error")))
        );
        for label in ["Cancel", "Discard changes", "Save changes"] {
            assert!(
                nodes
                    .iter()
                    .any(|(_, node)| node.role() == egui::accesskit::Role::Button
                        && node.label() == Some(label)
                        && !node.is_disabled())
            );
        }
    }
}

#[test]
fn replacing_existing_work_uses_an_explicit_warning() {
    let context = Context::default();
    context.enable_accesskit();
    let frame = || {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
                ..Default::default()
            },
            |ui| {
                DecisionDialog::new(
                    "Replace this exported file?",
                    "A file already exists at this location. Exporting will overwrite it.",
                    "Replace file",
                )
                .warning()
                .show(ui.ctx(), Id::new("test-warning-decision"), true);
            },
        );
        output.textures_delta.clear();
        output
    };
    frame();
    let output = frame();
    text(&output, "Warning");
    text(&output, "Replace this exported file?");
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    assert!(
        nodes
            .iter()
            .any(|(_, node)| node.value() == Some("Warning"))
    );
}

#[test]
fn backdrop_blocks_underlying_actions_without_dismissing_and_escape_cancels() {
    let context = Context::default();
    frame(&context, 800., vec![]);
    frame(&context, 800., vec![]);
    for pressed in [true, false] {
        let (choice, underlying, _) = frame(&context, 800., pointer(egui::pos2(30., 15.), pressed));
        assert_eq!(choice, None);
        assert!(!underlying);
    }
    assert_eq!(
        frame(&context, 800., key(egui::Key::Escape)).0,
        Some(DialogChoice::Cancel)
    );
}

#[test]
fn cancel_is_the_safe_initial_focus_and_explicit_pointer_choices_are_single_actions() {
    for (label, expected) in [
        ("Cancel", DialogChoice::Cancel),
        ("Discard changes", DialogChoice::Alternative),
        ("Save changes", DialogChoice::Confirm),
    ] {
        let context = Context::default();
        frame(&context, 800., vec![]);
        let (_, _, output) = frame(&context, 800., vec![]);
        assert_eq!(
            frame(&context, 800., key(egui::Key::Enter)).0,
            Some(DialogChoice::Cancel)
        );
        frame(
            &context,
            800.,
            vec![Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let point = text(&output, label).center();
        assert_eq!(frame(&context, 800., pointer(point, true)).0, None);
        assert_eq!(
            frame(&context, 800., pointer(point, false)).0,
            Some(expected)
        );
        assert_eq!(frame(&context, 800., vec![]).0, None);
    }
}

#[test]
fn reopening_after_pointer_cancel_does_not_retain_destructive_keyboard_focus() {
    let context = Context::default();
    frame(&context, 800., vec![]);
    let (_, _, output) = frame(&context, 800., vec![]);
    frame(&context, 800., key(egui::Key::Tab));
    let point = text(&output, "Cancel").center();
    assert_eq!(frame(&context, 800., pointer(point, true)).0, None);
    assert_eq!(
        frame(&context, 800., pointer(point, false)).0,
        Some(DialogChoice::Cancel)
    );
    assert_eq!(frame(&context, 800., vec![]).0, None);
    assert_eq!(
        frame(&context, 800., key(egui::Key::Enter)).0,
        Some(DialogChoice::Cancel)
    );
}

#[test]
fn reopening_after_keyboard_choice_restores_safe_initial_focus() {
    let context = Context::default();
    frame(&context, 800., vec![]);
    frame(&context, 800., vec![]);
    frame(&context, 800., key(egui::Key::Tab));
    assert_eq!(
        frame(&context, 800., key(egui::Key::Enter)).0,
        Some(DialogChoice::Alternative)
    );
    assert_eq!(frame(&context, 800., vec![]).0, None);
    assert_eq!(
        frame(&context, 800., key(egui::Key::Enter)).0,
        Some(DialogChoice::Cancel)
    );
}

#[test]
fn tab_follows_visual_action_order_and_explicit_focus_allows_keyboard_activation() {
    for width in [800., 260.] {
        for (tabs, expected) in [(1, DialogChoice::Alternative), (2, DialogChoice::Confirm)] {
            let context = Context::default();
            frame(&context, width, vec![]);
            frame(&context, width, vec![]);
            for _ in 0..tabs {
                assert_eq!(frame(&context, width, key(egui::Key::Tab)).0, None);
            }
            assert_eq!(
                frame(&context, width, key(egui::Key::Enter)).0,
                Some(expected)
            );
        }
    }
}

#[test]
fn a_popup_owns_escape_before_the_decision() {
    let context = Context::default();
    frame(&context, 800., vec![]);
    frame(&context, 800., vec![]);
    let popup = Id::new("decision-popup");
    egui::Popup::open_id(&context, popup);
    assert_eq!(frame(&context, 800., key(egui::Key::Escape)).0, None);
    egui::Popup::close_id(&context, popup);
    assert_eq!(
        frame(&context, 800., key(egui::Key::Escape)).0,
        Some(DialogChoice::Cancel)
    );
}
