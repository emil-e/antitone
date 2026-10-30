use super::{film_emulation::painted, workflow::enter_numeric, *};

#[test]
fn independent_halation_controls_persist_render_and_undo_at_both_display_scales() {
    editor_harness_with_gpu(true, |mut h, _, _, _| {
        h.enter(|editor, cx| editor.add_raws(vec![fixture("roll-0.dng")], cx));
        wait_for(&mut h, |e| {
            !e.busy && e.document.visit.canvas.ready.is_some()
        });
        stage(&mut h, "Rendering");
        assert!(h.query_by_label("Enable halation").is_none());
        h.enter(|e, cx| e.apply_input(crate::processing::InputSettings::Digital, cx));
        wait_for(&mut h, |e| {
            !e.busy && e.document.input == crate::processing::InputSettings::Digital
        });
        h.update(|e| {
            e.document.source = false;
            e.document.target = OutputTarget::Srgb;
            e.sync_document();
        });
        for scale in [1., 2.] {
            h.input_mut()
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            stage(&mut h, "Rendering");
            h.run();
            if !h.read(|e| e.document.halation.enabled) {
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable halation")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable halation")
                    .click();
                h.step();
            }
            if scale == 1. {
                h.read(|e| {
                    assert_eq!(e.document.halation.size, 0.109);
                    assert_eq!(e.document.halation.strength, 0.54);
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
                    !e.document.film_emulation.enabled,
                    "no MTF/grain master dependency"
                );
                assert!(e.document.halation.enabled);
                assert_eq!(e.document.halation.size, 0.103);
                assert_eq!(e.document.halation.strength, 0.37);
                assert_eq!(
                    e.document.state.project.output.halation,
                    e.document.halation
                );
                assert_eq!(e.document.target, OutputTarget::Srgb);
                let reopened: crate::project::Project = serde_json::from_str(
                    &serde_json::to_string(&e.document.state.project).unwrap(),
                )
                .unwrap();
                assert_eq!(reopened.output.halation, e.document.halation);
                assert_eq!(
                    e.positive_processing(
                        e.document.input,
                        e.document.roll,
                        e.document.visit.adjustment_layer_settings
                    )
                    .unwrap()
                    .halation(),
                    e.document.halation
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
                assert!(!e.document.halation.enabled);
                assert_eq!(e.document.halation.strength, 0.37);
            });
            h.enter(|e, cx| e.restore_history(super::super::history::Direction::Undo, cx));
            wait_for(&mut h, |e| !e.busy);
            h.read(|e| {
                assert!(e.document.halation.enabled);
                assert_eq!(e.document.halation.size, 0.103);
            });
            painted(&mut h);
            let png = h.render().unwrap();
            if scale == 1. {
                save_capture(&png, "halation-controls-offscreen");
            }
        }
    });
}
