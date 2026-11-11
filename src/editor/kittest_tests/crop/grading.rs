use super::*;

#[test]
fn accepted_grading_survives_crop_apply_cancel_and_tool_replacement() {
    for exit in ["Apply", "Cancel", "Dust repair", "Add linear"] {
        editor_harness(|mut h, _| {
            h.update(|editor| {
                editor.document.project.input = crate::processing::InputSettings::Digital;
            });
            h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
            wait_for(&mut h, |editor| {
                !editor.interaction_blocked() && editor.document.visit.canvas.ready.is_some()
            });
            stage(&mut h, "Photo");
            workflow::enter_numeric(&mut h, "Exposure", "0.75");
            h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.75));
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                .click();
            h.step();
            wait_for(&mut h, |editor| {
                editor
                    .document
                    .crop_edit()
                    .is_some_and(|edit| edit.layout.is_some())
            });
            let initial = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            drag(
                &mut h,
                initial.image.min,
                initial.image.min + egui::vec2(25., 18.),
            );
            let draft = h.read(|editor| editor.document.crop_edit().unwrap().draft);
            assert_ne!(
                draft,
                initial.composition.crop(),
                "fixture must have an actual unaccepted edit"
            );
            adjustment_layers::show_layers(&mut h);
            h.event(egui::Event::PointerGone);
            h.step();
            h.render().unwrap();
            let exposure = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure");
            exposure.scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            h.read(|editor| {
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert!(editor.photo_crop().is_none());
                assert_eq!(editor.photo_settings().exposure, 0.75);
            });
            let mut expected = h.read(|editor| editor.document.state.project.clone());
            let source = h.read(|editor| editor.document.visit.current_source.unwrap());
            if exit == "Apply" {
                expected.source_mut(source).unwrap().crop = Some(draft);
            }
            h.get_by_role_and_label(egui::accesskit::Role::Button, exit)
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            let action = h.get_by_role_and_label(egui::accesskit::Role::Button, exit);
            assert!(!action.accesskit_node().is_disabled());
            action.click();
            h.step();
            wait_for(&mut h, |editor| {
                !editor.interaction_blocked() && !editor.cropping()
            });
            h.read(|editor| {
                assert_eq!(editor.document.state.project, expected);
                assert_eq!(editor.photo_settings().exposure, 0.75);
                if exit == "Dust repair" {
                    assert!(editor.dusting());
                }
                if exit == "Add linear" {
                    assert!(editor.creating_mask());
                }
            });
            if exit == "Dust repair" {
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Done")
                    .click();
                h.step();
            } else if exit == "Add linear" {
                h.key_press(egui::Key::Escape);
                h.step();
            }
            if exit == "Apply" {
                h.read(|editor| {
                    assert_eq!(
                        editor.history_label(super::super::super::history::Direction::Undo),
                        Some("Crop")
                    )
                });
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
                h.step();
                wait_for(&mut h, |editor| !editor.interaction_blocked());
                h.read(|editor| {
                    assert!(editor.photo_crop().is_none());
                    assert_eq!(editor.photo_settings().exposure, 0.75);
                });
            }
            h.read(|editor| {
                assert_eq!(
                    editor.history_label(super::super::super::history::Direction::Undo),
                    Some("Photo settings")
                )
            });
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.step();
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.));
        });
    }
}
