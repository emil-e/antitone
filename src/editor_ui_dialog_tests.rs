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
fn decisions_keep_text_and_distinct_actions_visible_at_narrow_widths() {
    for width in [1100., 800., 320., 260.] {
        let context = Context::default();
        frame(&context, width, vec![]);
        frame(&context, width, vec![]);
        let (_, _, output) = frame(&context, width, vec![]);
        let area = context.memory(|memory| memory.area_rect(Id::new("test-decision")).unwrap());
        let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.));
        assert!(viewport.contains_rect(area), "{area:?}");
        for label in [
            "Save changes to this project?",
            "Opening another project without saving will discard unsaved edits in this project.",
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
        }
        let cancel = text(&output, "Cancel");
        let alternative = text(&output, "Discard changes");
        let confirm = text(&output, "Save changes");
        let detail = text(
            &output,
            "/projects/照片-very-long-project-with-no-spaces-that-must-wrap.antitone.json",
        );
        for action in [cancel, alternative, confirm] {
            assert!(!action.intersects(detail));
        }
        assert!(!cancel.intersects(alternative));
        assert!(!cancel.intersects(confirm));
        assert!(!alternative.intersects(confirm));
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        assert!(
            !nodes
                .iter()
                .any(|(_, node)| matches!(node.value(), Some("Warning" | "Information" | "Error")))
        );
        let mut hit_regions = Vec::new();
        for label in ["Cancel", "Discard changes", "Save changes"] {
            let action = &nodes
                .iter()
                .find(|(_, node)| {
                    node.role() == egui::accesskit::Role::Button && node.label() == Some(label)
                })
                .unwrap()
                .1;
            assert!(!action.is_disabled());
            let bounds = action.bounds().unwrap();
            let hit = Rect::from_min_max(
                egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
            );
            assert!(area.contains_rect(hit));
            assert!(hit.contains_rect(text(&output, label)));
            assert!(!hit.intersects(detail));
            assert!(hit_regions.iter().all(|other| !hit.intersects(*other)));
            hit_regions.push(hit);
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
