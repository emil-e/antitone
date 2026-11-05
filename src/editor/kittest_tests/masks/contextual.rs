use super::*;

fn handle(editor: &Editor) -> egui::Pos2 {
    let point = match accepted_mask(editor) {
        AdjustmentLayerCoverage::Linear { end, .. } => end,
        AdjustmentLayerCoverage::Radial { center, .. } => center,
        AdjustmentLayerCoverage::Global {} => unreachable!(),
    };
    let layout = editor.active_canvas().last_layout.unwrap().0;
    layout.screen_frame_point(layout.composition.source_to_frame(point))
}

fn no_mode(h: &Harness<'_, Editor>) {
    h.read(|editor| {
        assert!(editor.current_tool().is_none());
        assert!(!editor.tool_interacting());
    });
    for label in ["Apply", "Cancel"] {
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, label)
                .is_none(),
            "{label}"
        );
    }
    assert!(
        !h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .accesskit_node()
            .is_disabled()
    );
}

fn history(h: &mut Harness<'_, Editor>, redo: bool) {
    h.key_press_modifiers(
        if redo {
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::COMMAND
        },
        egui::Key::Z,
    );
    h.step();
    wait_for(h, |editor| !editor.interaction_blocked());
}

#[test]
fn grading_and_contextual_refinement_alternate_without_a_mode_and_undo_independently() {
    for (radial, size, width) in [
        (false, egui::vec2(1100., 800.), 315.),
        (true, egui::vec2(800., 600.), 270.),
    ] {
        workflow::calibrated_workflow(|mut h, _, _directory| {
            set_layout(&mut h, size, width);
            arm(&mut h, radial);
            let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
            drag(
                &mut h,
                image.center() - egui::vec2(60., 35.),
                image.center() + egui::vec2(60., 35.),
            );
            workflow::enter_numeric(&mut h, "Exposure", "0.25");
            no_mode(&h);
            let graded = h.read(|editor| editor.document.state.project.clone());
            let original_mask = h.read(accepted_mask);
            let start = h.read(handle);
            let end = start + egui::vec2(24., -12.);
            hold_drag(&mut h, start, end);
            no_mode(&h);
            h.read(|editor| {
                assert!(editor.document.visit.masks.dragging());
                assert_eq!(editor.document.state.project, graded);
                assert!(editor.mask_preview_source().is_some());
            });
            save_capture(
                &h.render().unwrap(),
                if radial {
                    "contextual-radial-drag"
                } else {
                    "contextual-linear-drag"
                },
            );
            pointer(&mut h, end, false);
            no_mode(&h);
            let refined = h.read(|editor| editor.document.state.project.clone());
            assert_ne!(h.read(accepted_mask), original_mask);
            assert_ne!(refined, graded);
            workflow::enter_numeric(&mut h, "Exposure", "0.75");
            let regraded = h.read(|editor| editor.document.state.project.clone());
            assert_ne!(regraded, refined);
            let start = h.read(handle);
            let end = start + egui::vec2(-18., 16.);
            drag(&mut h, start, end);
            no_mode(&h);
            let refined_again = h.read(|editor| editor.document.state.project.clone());
            assert_ne!(refined_again, regraded);

            for expected in [&regraded, &refined, &graded] {
                history(&mut h, false);
                h.read(|editor| assert_eq!(&editor.document.state.project, expected));
            }
            let start = h.read(handle);
            let end = start + egui::vec2(14., 10.);
            hold_drag(&mut h, start, end);
            h.read(|editor| assert!(editor.document.visit.masks.dragging()));
            h.key_press(egui::Key::Escape);
            h.step();
            pointer(&mut h, end, false);
            h.read(|editor| assert_eq!(editor.document.state.project, graded));
            for expected in [&refined, &regraded, &refined_again] {
                history(&mut h, true);
                h.read(|editor| assert_eq!(&editor.document.state.project, expected));
            }
            no_mode(&h);
        });
    }
}

#[test]
fn refinement_paints_its_draft_without_changing_the_comparison_preference() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        arm(&mut h, false);
        let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
        drag(
            &mut h,
            image.center() - egui::vec2(60., 35.),
            image.center() + egui::vec2(60., 35.),
        );
        click(&mut h, "Original-resolution reference");
        workflow::wait_settled(&mut h);
        h.update(|editor| {
            assert!(editor.comparing_original());
            let reference = editor
                .original_rendered(OriginalPurpose::Reference)
                .unwrap()
                .unwrap();
            assert!(Arc::ptr_eq(
                &editor.canvas_frame.displayed.as_ref().unwrap().view.image,
                &reference.image
            ));
        });
        let before = h.read(|editor| editor.document.state.project.clone());
        let start = h.read(handle);
        let end = start + egui::vec2(24., -12.);
        hold_drag(&mut h, start, end);
        let wanted = h.read(|editor| {
            assert!(editor.document.visit.masks.dragging());
            assert!(editor.document.visit.working_view.compare);
            assert!(!editor.comparing_original());
            assert_eq!(editor.document.state.project, before);
            crate::gpu::photograph::request::PhotographRequest::Recipe(Arc::new(
                editor
                    .view_recipe(
                        editor.document.input,
                        editor.document.roll,
                        editor.document.visit.adjustment_layer_settings,
                    )
                    .unwrap(),
            ))
        });
        wait_for(&mut h, |editor| {
            editor
                .canvas_frame
                .displayed
                .as_ref()
                .is_some_and(|painted| painted.view.capture == wanted)
        });
        save_capture(&h.render().unwrap(), "contextual-refinement-over-reference");
        h.key_press(egui::Key::Escape);
        h.step();
        pointer(&mut h, end, false);
        workflow::wait_settled(&mut h);
        h.read(|editor| {
            assert!(editor.comparing_original());
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn departing_photo_and_tool_replacement_cannot_publish_a_held_refinement() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        arm(&mut h, false);
        let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
        drag(
            &mut h,
            image.center() - egui::vec2(60., 35.),
            image.center() + egui::vec2(60., 35.),
        );
        workflow::enter_numeric(&mut h, "Exposure", "0.5");
        h.update(|editor| editor.document.visit.working_view.compare = true);
        let before = h.read(|editor| editor.document.state.project.clone());
        let before_history =
            h.read(|editor| editor.history_label(crate::editor::history::Direction::Undo));
        let first = h.read(|editor| editor.document.visit.current_source.unwrap());
        let start = h.read(handle);
        let end = start + egui::vec2(25., -10.);
        hold_drag(&mut h, start, end);
        h.read(|editor| {
            assert!(editor.document.visit.masks.dragging());
            assert!(editor.document.visit.working_view.compare);
            assert!(!editor.comparing_original());
        });
        let next = h.read(|editor| {
            let id = editor.document.project.order[1];
            editor.document.source_path(id).unwrap()
        });
        h.enter(|editor, cx| editor.load(next, cx));
        pointer(&mut h, end, false);
        wait_for(&mut h, |editor| {
            !editor.document.visit.source.pending() && editor.active_canvas().last_layout.is_some()
        });
        h.read(|editor| {
            assert_ne!(editor.document.visit.current_source, Some(first));
            assert!(!editor.document.visit.masks.editing());
            assert!(editor.document.visit.working_view.compare);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                before_history
            );
        });
        h.get_by_label("working-photo.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            !editor.document.visit.source.pending() && editor.active_canvas().last_layout.is_some()
        });
        let start = h.read(handle);
        let end = start + egui::vec2(20., 15.);
        hold_drag(&mut h, start, end);
        h.read(|editor| assert!(editor.document.visit.masks.dragging()));
        h.update(Editor::begin_crop);
        pointer(&mut h, end, false);
        h.read(|editor| {
            assert!(editor.cropping());
            assert!(!editor.document.visit.masks.editing());
            assert_eq!(editor.document.state.project, before);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                before_history
            );
        });
        click(&mut h, "Cancel");
        no_mode(&h);
    });
}
