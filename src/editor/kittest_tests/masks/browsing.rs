use super::*;
use crate::editor::{
    history::Direction,
    kittest_tests::tool_browsing::{depart, return_to_photo},
};

#[test]
fn armed_gradient_creation_ends_on_pointer_or_keyboard_photo_departure() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            for radial in [false, true] {
                for keyboard in [false, true] {
                    let before = harness.read(|editor| editor.document.state.project.clone());
                    let history = harness.read(|editor| editor.history_label(Direction::Undo));
                    arm(&mut harness, radial);
                    harness.read(|editor| assert!(editor.creating_mask()));
                    harness.get_by_label("working-photo.dng").click();
                    harness.step();
                    harness.read(|editor| assert!(editor.creating_mask()));
                    depart(&mut harness, keyboard);
                    harness.read(|editor| {
                        assert!(!editor.masking() && !editor.has_hosted_tool());
                        assert_eq!(editor.document.state.project, before);
                        assert_eq!(editor.history_label(Direction::Undo), history);
                    });
                    return_to_photo(&mut harness);
                    harness.read(|editor| assert!(!editor.masking()));
                }
            }
        }
    });
}

#[test]
fn keyboard_departure_discards_held_creation_and_refinement_before_a_late_release() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            for radial in [false, true] {
                for refining in [false, true] {
                    arm(&mut harness, radial);
                    let image =
                        harness.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
                    let first = image.center() - egui::vec2(35., 25.);
                    let last = image.center() + egui::vec2(35., 25.);
                    let start = if refining {
                        drag(&mut harness, first, last);
                        harness.read(|editor| assert!(!editor.creating_mask()));
                        harness.read(|editor| {
                            let source = match accepted_mask(editor) {
                                AdjustmentLayerCoverage::Linear { end, .. } => end,
                                AdjustmentLayerCoverage::Radial { center, .. } => center,
                                AdjustmentLayerCoverage::Global {} => unreachable!(),
                            };
                            let layout = editor.active_canvas().last_layout.unwrap().0;
                            layout.screen_frame_point(layout.composition.source_to_frame(source))
                        })
                    } else {
                        first
                    };
                    let before = harness.read(|editor| editor.document.state.project.clone());
                    let history = harness.read(|editor| editor.history_label(Direction::Undo));
                    let end = start + egui::vec2(25., 20.);
                    hold_drag(&mut harness, start, end);
                    harness.read(|editor| {
                        assert!(editor.mask_dragging());
                        assert_eq!(editor.document.visit.masks.dragging(), refining);
                    });
                    depart(&mut harness, true);
                    pointer(&mut harness, end, false);
                    harness.try_run_realtime().unwrap();
                    harness.read(|editor| {
                        assert!(!editor.masking());
                        assert_eq!(editor.document.state.project, before);
                        assert_eq!(editor.history_label(Direction::Undo), history);
                    });
                    return_to_photo(&mut harness);
                    harness.read(|editor| {
                        assert!(!editor.masking());
                        assert_eq!(editor.document.state.project, before);
                    });
                }
            }
        }
    });
}
