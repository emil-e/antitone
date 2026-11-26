use super::*;
use crate::editor::history::Direction;
use std::sync::{Mutex, mpsc};

fn calibrated_roll(harness: &mut Harness<'_, Editor>) -> Vec<SourceId> {
    harness.enter(|editor, cx| {
        editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
    });
    wait_for(harness, |editor| !editor.interaction_blocked());
    harness.update(|editor| {
        editor.set_dmin(Some(DVec3::new(0.2, 0.15, 0.1)));
        editor.set_dmax(Some(DVec3::new(0.008, 0.005, 0.002)));
    });
    harness.step();
    harness.read(|editor| editor.document.project.order.clone())
}

#[test]
fn review_uses_ordinary_selection_without_retargeting_its_roll_wide_drafts() {
    editor_harness(|mut harness, _| {
        let ids = calibrated_roll(&mut harness);
        show_confirmation(&mut harness);
        let before = harness.read(|editor| editor.document.project.clone());
        let drafts = harness.read(|editor| {
            editor
                .document
                .analysis_review()
                .unwrap()
                .approved
                .entries
                .clone()
        });
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            harness
                .get_by_label("roll-1.dng")
                .click_modifiers(egui::Modifiers::COMMAND);
            harness.step();
            harness.read(|editor| {
                assert_eq!(editor.document.visit.current_source, Some(ids[0]));
                assert!(
                    ids.iter()
                        .all(|id| editor.document.roll_selection.contains(*id))
                );
            });
            harness.get_by_label("roll-0.dng").focus();
            harness.step();
            harness.key_press_modifiers(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::A,
            );
            harness.step();
            harness.read(|editor| {
                assert!(editor.document.roll_selection.contains(ids[0]));
                assert!(!editor.document.roll_selection.contains(ids[1]));
            });
            harness.get_by_label("roll-1.dng").click();
            harness.step();
            wait_for(&mut harness, |editor| !editor.interaction_blocked());
            harness.read(|editor| {
                assert_eq!(editor.document.visit.current_source, Some(ids[1]));
                assert!(!editor.document.roll_selection.contains(ids[0]));
                assert!(editor.document.roll_selection.contains(ids[1]));
            });
            harness.key_press(egui::Key::ArrowLeft);
            harness.step();
            wait_for(&mut harness, |editor| !editor.interaction_blocked());
            harness.read(|editor| {
                assert_eq!(editor.document.visit.current_source, Some(ids[0]));
                assert!(editor.document.roll_selection.contains(ids[0]));
                assert!(!editor.document.roll_selection.contains(ids[1]));
                assert_eq!(
                    editor.document.analysis_review().unwrap().approved.entries,
                    drafts
                );
                assert_eq!(editor.document.project, before);
            });
        }
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| {
            assert!(editor.current_tool().is_none());
            assert_eq!(editor.document.project, before);
        });
    });
}

#[test]
fn starting_analysis_with_enter_blocks_roll_navigation_in_the_same_pass() {
    editor_harness(|mut h, _| {
        let ids = calibrated_roll(&mut h);
        show_confirmation(&mut h);
        h.read(|editor| assert_eq!(editor.document.visit.current_source, Some(ids[0])));
        ui::surrender_keyboard_focus(&h.ctx);

        h.key_press(egui::Key::Enter);
        h.key_press(egui::Key::ArrowRight);
        h.step();
        h.read(|editor| {
            assert!(editor.analysis_fitting() && editor.workspace_input.is_blocked());
            assert_eq!(editor.document.visit.current_source, Some(ids[0]));
        });
    });
}

#[test]
fn reordering_during_review_preserves_sampling_areas_and_accepts_in_the_new_order() {
    editor_harness(|mut h, _| {
        let ids = calibrated_roll(&mut h);
        show_confirmation(&mut h);
        let reviewed: Vec<_> = h.read(|editor| {
            editor
                .document
                .analysis_review()
                .unwrap()
                .approved
                .records()
                .collect()
        });
        let start = h.get_by_label("roll-0.dng").rect().center();
        let end = h.get_by_label("roll-1.dng").rect().right_center() - egui::vec2(4., 0.);
        drag(&mut h, start, end);
        h.read(|editor| {
            assert_eq!(editor.document.project.order, [ids[1], ids[0]]);
            assert_eq!(editor.document.visit.current_source, Some(ids[0]));
            let review = editor.document.analysis_review().unwrap();
            assert!(review.approved.records().eq(reviewed.iter().cloned()));
        });
        let apply = h
            .get_all_by_role_and_label(egui::accesskit::Role::Button, "Analyze roll")
            .find(|node| !node.accesskit_node().is_disabled())
            .expect("enabled review confirmation");
        apply.click();
        h.step();
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert!(
                editor.document.analysis_review().is_none(),
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project.order, [ids[1], ids[0]]);
            assert_eq!(editor.document.project.analysis_regions, reviewed);
            assert_eq!(editor.document.visit.current_source, Some(ids[0]));
            assert_eq!(editor.history_label(Direction::Undo), Some("Analyze roll"));
        });
    });
}

#[test]
fn fullscreen_keeps_source_review_editing_and_its_navigation() {
    editor_harness(|mut harness, _| {
        calibrated_roll(&mut harness);
        show_confirmation(&mut harness);
        for (name, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            let image = harness.read(|editor| editor.canvas_frame.layout.unwrap().0.image);
            drag(
                &mut harness,
                image.min + image.size() * 0.3,
                image.min + image.size() * 0.7,
            );
            harness.get_by_label("100%").click();
            harness.step();
            let (drafts, normal, project, preferences) = harness.read(|editor| {
                (
                    editor
                        .document
                        .analysis_review()
                        .unwrap()
                        .approved
                        .entries
                        .clone(),
                    editor.document.visit.canvas.viewport,
                    editor.document.project.clone(),
                    editor.preferences.clone(),
                )
            });
            assert!(
                !harness
                    .get_by_label("Fullscreen viewing")
                    .accesskit_node()
                    .is_disabled()
            );
            harness.get_by_label("Fullscreen viewing").click();
            harness.step();
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert!(editor.fullscreen.is_some());
                let painted = editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .expect("fullscreen source painted");
                assert!(painted.view.capture.is_inspection());
                assert!(painted.view.look_id().is_none());
            });
            assert!(harness.query_by_label("Analyze roll").is_some());
            assert!(harness.query_by_label("Cancel").is_some());
            let corner = harness.read(|editor| {
                let source = editor.document.visit.current_source.unwrap();
                let entry = editor
                    .document
                    .analysis_review()
                    .unwrap()
                    .approved
                    .entries
                    .iter()
                    .find(|entry| entry.source == source)
                    .unwrap();
                let point = entry.areas.picture.unwrap()[0];
                editor
                    .canvas_frame
                    .layout
                    .unwrap()
                    .0
                    .screen_point(egui::pos2(point[0] as f32, point[1] as f32))
            });
            drag(&mut harness, corner, corner + egui::vec2(20., 15.));
            harness.key_press(egui::Key::Z);
            harness.step();
            harness.update(|editor| {
                editor.with_tool_viewport(|viewport| assert!(viewport.zoom.is_none()))
            });
            let edited = harness.read(|editor| {
                assert!(!editor.analysis_fitting());
                let entries = editor
                    .document
                    .analysis_review()
                    .unwrap()
                    .approved
                    .entries
                    .clone();
                assert_ne!(entries, drafts, "fullscreen selection must edit the review");
                assert_eq!(editor.document.project, project);
                entries
            });
            save_capture(
                &harness.render().unwrap(),
                &format!("analyze-fullscreen-{name}-offscreen"),
            );
            harness.key_press(egui::Key::Escape);
            harness.step();
            harness.read(|editor| {
                assert!(
                    editor.fullscreen.is_some(),
                    "await real host acknowledgement"
                )
            });
            harness.update(|editor| editor.fullscreen_changed(false));
            harness.step();
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert!(editor.fullscreen.is_none());
                assert_eq!(
                    editor.document.analysis_review().unwrap().approved.entries,
                    edited
                );
                assert!(editor.document.visit.analysis_view.viewport.zoom.is_none());
                assert_eq!(editor.document.visit.canvas.viewport, normal);
                assert_eq!(editor.document.project, project);
                assert_eq!(editor.preferences, preferences);
            });
        }
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert!(editor.current_tool().is_none()));
    });
}

struct Release(mpsc::Sender<()>);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn review_and_fitting_cancel_during_observed_loading_keep_the_photo_request_and_ordinary_view_preference()
 {
    for (source, fitting) in [(false, false), (true, false), (false, true), (true, true)] {
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let release = Release(release);
        let resume = Mutex::new(Some(resume));
        let sources = source_loading::SourceLoader::new_observed(move |path, phase, _| {
            if path.file_name().is_some_and(|name| name == "roll-1.dng")
                && matches!(phase, source_loading::NativePhase::Open)
                && let Some(resume) = resume.lock().unwrap().take()
            {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(60)).unwrap();
            }
            Ok(())
        });
        let original = editor_harness(|mut harness, _| {
            let release = release;
            let ids = calibrated_roll(&mut harness);
            harness.update(|editor| editor.document.source = source);
            harness.get_by_label("roll-1.dng").click();
            harness.step();
            wait_for(&mut harness, |editor| !editor.interaction_blocked());
            let ordinary_source = harness.read(|editor| editor.document.source);
            harness.get_by_label("roll-0.dng").click();
            harness.step();
            wait_for(&mut harness, |editor| !editor.interaction_blocked());
            harness.update(|editor| editor.document.source = source);
            show_confirmation(&mut harness);
            harness.read(|editor| assert_eq!(editor.document.source, source));
            let before = harness.read(|editor| editor.document.project.clone());
            let history =
                harness.read(|editor| editor.history_label(Direction::Undo).map(str::to_owned));
            let original = harness.update(|editor| {
                editor.document.cache.set_budget(0);
                std::mem::replace(
                    &mut editor.source_services.as_mut().unwrap().developments,
                    sources.clone(),
                )
            });
            harness.get_by_label("roll-1.dng").click();
            harness.step();
            wait_for(&mut harness, |_| starts.try_recv().is_ok());
            let (request, stopped) = harness.read(|editor| {
                assert!(!editor.interaction_blocked() && editor.document.visit.source.pending());
                assert_eq!(editor.document.visit.current_source, Some(ids[1]));
                assert_eq!(editor.document.source, ordinary_source);
                (
                    editor.visit_lifetime(),
                    editor
                        .document
                        .analysis_operation()
                        .unwrap()
                        .cancelled
                        .clone(),
                )
            });
            assert!(
                !harness
                    .get_by_label("Analyze roll")
                    .accesskit_node()
                    .is_disabled(),
                "review confirmation can acquire precise inputs without waiting for the selected display",
            );
            assert!(
                !harness
                    .get_by_label("Cancel")
                    .accesskit_node()
                    .is_disabled()
            );
            for photo in ["roll-0.dng", "roll-1.dng"] {
                assert!(
                    !harness.get_by_label(photo).accesskit_node().is_disabled(),
                    "review browsing remains available during a pending photo request: {photo}"
                );
            }

            save_capture(
                &harness.render().unwrap(),
                &format!("analyze-browsing-cancel-source-{source}-offscreen"),
            );

            if fitting {
                harness.get_by_label("Analyze roll").click();
                harness.step();

                harness.update(|editor| editor.activities.elapse(Duration::from_secs(1)));
                harness.run_steps(3);
                harness.read(|editor| {
                    assert!(editor.analysis_fitting() && editor.document.visit.source.pending());
                    assert!(editor.interaction_blocked());
                });

                harness.get_by_label("Cancel analysis").click();
            } else {
                harness.get_by_label("Cancel").click();
            }
            harness.step();
            assert!(
                stopped.is_cancelled(),
                "the modal or review Cancel control must receive the click"
            );
            wait_for(&mut harness, |editor| {
                editor.document.analysis_operation().is_none()
            });

            harness.read(|editor| {
                assert!(stopped.is_cancelled());
                assert!(editor.current_tool().is_none());
                assert!(!editor.interaction_blocked() && editor.document.visit.source.pending());
                assert!(request.upgrade().is_some());
                assert_eq!(editor.activities.view().unwrap().title, "Loading photo…");
                assert_eq!(editor.document.project, before);
                assert_eq!(editor.document.source, ordinary_source);
            });
            drop(release);
            wait_for(&mut harness, |editor| {
                !editor.interaction_blocked() && editor.document.visit.canvas.ready.is_some()
            });
            harness.read(|editor| {
                assert_eq!(editor.document.visit.current_source, Some(ids[1]));
                assert_eq!(editor.document.project, before);
                assert_eq!(editor.history_label(Direction::Undo), history.as_deref());
                assert_eq!(editor.document.source, ordinary_source);
            });
            original
        });
        original.close_and_wait().unwrap();
        sources.close_and_wait().unwrap();
    }
}

#[test]
fn selected_mask_artwork_stays_out_of_the_source_review_during_moved_out_painting() {
    fn artwork(editor: &Editor) -> usize {
        let context = egui::Context::default();
        let mut output = context.run_ui(Default::default(), |ui| {
            editor.paint_mask(ui, editor.canvas_frame.layout.unwrap().0);
        });
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter(|shape| !matches!(shape.shape, egui::Shape::Noop))
            .count()
    }
    editor_harness(|mut harness, _| {
        calibrated_roll(&mut harness);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
            .click();
        harness.run_steps(2);
        harness.get_by_label("Add linear").scroll_to_me();
        harness.run_steps(2);
        harness.get_by_label("Add linear").click();
        wait_for(&mut harness, |editor| editor.canvas_frame.layout.is_some());
        let image = harness.read(|editor| editor.canvas_frame.layout.unwrap().0.image);
        drag(
            &mut harness,
            image.min + image.size() * 0.25,
            image.min + image.size() * 0.75,
        );
        harness.read(|editor| {
            assert!(!editor.creating_mask());
            assert!(
                artwork(editor) > 0,
                "accepted selected gradient must paint before review"
            );
        });
        harness
            .get_by_role_and_label(egui::accesskit::Role::Tab, "Roll")
            .click();
        harness.run_steps(2);
        show_confirmation(&mut harness);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
            .click();
        harness.run_steps(2);
        harness.update(|editor| {
            editor
                .with_current_tool(|_, editor| {
                    assert_eq!(
                        artwork(editor),
                        0,
                        "source review must hide selected-mask painting while its box is moved out"
                    );
                })
                .unwrap();
        });
        harness.get_by_label("Cancel").click();
        wait_for(&mut harness, |editor| editor.canvas_frame.layout.is_some());
        harness.read(|editor| {
            assert_eq!(
                artwork(editor),
                0,
                "a retired mask tool does not resume implicitly"
            );
        });
        adjustment_layers::show_layers(&mut harness);
        let name = harness.read(|editor| {
            adjustment_layers::layers(editor)
                .layers
                .last()
                .unwrap()
                .name
                .clone()
        });
        adjustment_layers::click(&mut harness, &format!("Edit {name}"));
        harness.read(|editor| {
            assert!(
                artwork(editor) > 0,
                "explicit Edit restores the unchanged selected gradient"
            )
        });
    });
}
