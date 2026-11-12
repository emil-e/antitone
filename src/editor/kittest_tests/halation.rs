use super::{grain::painted, workflow::enter_numeric, *};

#[test]
fn independent_halation_controls_persist_render_and_undo_at_both_display_scales() {
    editor_harness_with_gpu(true, |mut h, _, _, _| {
        h.enter(|editor, cx| editor.add_raws(vec![fixture("roll-0.dng")], cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked() && e.document.visit.canvas.ready.is_some()
        });
        stage(&mut h, "Rendering");
        assert!(h.query_by_label("Enable halation").is_none());
        h.enter(|e, cx| e.apply_input(crate::processing::InputSettings::Digital, cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked()
                && e.document.project.input == crate::processing::InputSettings::Digital
        });
        h.update(|e| {
            e.document.source = false;
            e.document.set_target(OutputTarget::Srgb);
        });
        for scale in [1., 2.] {
            h.input_mut()
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            stage(&mut h, "Rendering");
            h.run();
            if !h.read(|e| e.document.project.output.halation.enabled) {
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable halation")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable halation")
                    .click();
                h.step();
            }
            if scale == 1. {
                for label in ["Core size", "Strength"] {
                    let role = egui::accesskit::Role::Slider;
                    h.get_by_role_and_label(role, label).scroll_to_me();
                    h.try_run_realtime().unwrap();
                    let control = h.get_by_role_and_label(role, label);
                    let node = control.accesskit_node();
                    let start = node.min_numeric_value().unwrap();
                    let end = node.max_numeric_value().unwrap();
                    let position = (node.numeric_value().unwrap() - start) / (end - start);
                    assert!(
                        (0.45..=0.55).contains(&position),
                        "{label} default stays near the centre of its slider"
                    );
                }
                h.read(|e| {
                    assert_eq!(e.document.project.output.halation.size, 0.109);
                    assert_eq!(e.document.project.output.halation.strength, 0.54);
                });
                painted(&mut h);
                save_capture(&h.render().unwrap(), "halation-defaults-offscreen");
            }
            enter_numeric(&mut h, "Core size", "0.400");
            h.try_run_realtime().unwrap();
            h.read(|e| assert_eq!(e.document.state.project.output.halation.size, 0.4));
            enter_numeric(&mut h, "Core size", "0.103");
            h.try_run_realtime().unwrap();
            enter_numeric(&mut h, "Strength", "0.37");
            h.try_run_realtime().unwrap();
            painted(&mut h);
            h.read(|e| {
                assert!(
                    !e.document.project.output.grain.enabled,
                    "no MTF/grain master dependency"
                );
                assert!(e.document.project.output.halation.enabled);
                assert_eq!(e.document.project.output.halation.size, 0.103);
                assert_eq!(e.document.project.output.halation.strength, 0.37);
                assert_eq!(e.document.target(), OutputTarget::Srgb);
                let reopened: crate::project::Project = serde_json::from_str(
                    &serde_json::to_string(&e.document.state.project).unwrap(),
                )
                .unwrap();
                assert_eq!(reopened.output.halation, e.document.project.output.halation);
                assert_eq!(
                    e.positive_processing(
                        e.document.project.input,
                        e.document.project.roll,
                        e.photo_settings()
                    )
                    .unwrap()
                    .pixel_pipeline()
                    .stages
                    .iter()
                    .find_map(|stage| stage.get::<crate::halation::HalationParameters>())
                    .copied(),
                    e.document
                        .project
                        .output
                        .halation
                        .parameters(
                            e.document
                                .state
                                .project
                                .image_recipe(e.document.visit.current_source.unwrap())
                                .unwrap()
                                .composition
                                .crop()
                                .size
                        )
                        .unwrap()
                );
            });
            h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable halation")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable halation")
                .click();
            h.step();
            h.try_run_realtime().unwrap();
            h.read(|e| {
                assert!(!e.document.project.output.halation.enabled);
                assert_eq!(e.document.project.output.halation.strength, 0.37);
            });
            h.enter(|e, cx| e.restore_history(super::super::history::Direction::Undo, cx));
            wait_for(&mut h, |e| !e.interaction_blocked());
            h.read(|e| {
                assert!(e.document.project.output.halation.enabled);
                assert_eq!(e.document.project.output.halation.size, 0.103);
            });
            painted(&mut h);
            let png = h.render().unwrap();
            if scale == 1. {
                save_capture(&png, "halation-controls-offscreen");
            }
        }
    });
}
