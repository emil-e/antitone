use super::*;

#[test]
fn native_and_keyboard_history_keep_each_real_picker_and_its_selected_region() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut h, size, width);
            for kind in [
                PickerCase::Dmin,
                PickerCase::Dmax,
                PickerCase::Gamma,
                PickerCase::GammaPivot,
                PickerCase::RollWb,
                PickerCase::Wb,
            ] {
                let before = h.read(|editor| editor.document.state.project.clone());
                h.update(|editor| {
                    let mut edit = editor.edit_project("Grain").unwrap();
                    edit.update(|project| project.output.grain.amount += 0.01);
                    editor.commit_project(edit).unwrap();
                });
                let after = h.read(|editor| editor.document.state.project.clone());
                open(&mut h, kind);
                let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
                drag(
                    &mut h,
                    image.center() - egui::vec2(10., 10.),
                    image.center() + egui::vec2(10., 10.),
                );
                let region = h.read(|editor| editor.test_pick_region().unwrap());
                for (action, expected) in [
                    (crate::platform::menu::Action::Undo, &before),
                    (crate::platform::menu::Action::Redo, &after),
                ] {
                    if action == crate::platform::menu::Action::Undo {
                        h.update(|editor| host::App::menu_action(editor, action));
                    } else {
                        h.key_press_modifiers(
                            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                            egui::Key::Z,
                        );
                    }
                    h.step();
                    h.try_run_realtime().unwrap();
                    h.read(|editor| {
                        assert_eq!(&editor.document.state.project, expected);
                        assert_eq!(editor.test_pick(), Some(kind.display()));
                        assert_eq!(editor.test_pick_region(), Some(region));
                        assert!(!editor.measurement_pending());
                    });
                    assert!(!h.get_by_label("Apply").accesskit_node().is_disabled());
                }
                h.get_by_label("Cancel").click();
                h.step();
                h.try_run_realtime().unwrap();
                h.read(|editor| assert!(!editor.picking()));
            }
        }
    });
}
