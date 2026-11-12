use super::*;
use crate::platform::menu::Action;

#[test]
fn scoped_input_blocks_pointer_keyboard_and_native_commands_without_a_feature_catalogue() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        harness.enter(|editor, cx| {
            editor.add_raws(vec![fixture("patches.dng"), fixture("roll-0.dng")], cx);
        });
        wait_opened_photo(&mut harness);
        harness.update(host::App::native_menu_installed);
        stage(&mut harness, "Photo");

        for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
            harness.set_size(size);
            let crop = egui::accesskit::Role::Button;
            harness
                .get_by_role_and_label(crop, "Crop & straighten")
                .scroll_to_me();
            harness.try_run_realtime().unwrap();
            assert!(
                !harness
                    .get_by_role_and_label(crop, "Crop & straighten")
                    .accesskit_node()
                    .is_disabled()
            );
            let before = harness.read(|editor| editor.document.state.project.clone());
            let selected = harness.read(|editor| editor.document.visit.current_source);
            let first = harness.update(|editor| editor.workspace_input.block());
            let second = harness.update(|editor| editor.workspace_input.block());
            harness.step();
            assert!(
                harness
                    .get_by_role_and_label(crop, "Crop & straighten")
                    .accesskit_node()
                    .is_disabled()
            );
            harness
                .get_by_role_and_label(crop, "Crop & straighten")
                .click();
            harness.key_press(egui::Key::ArrowRight);
            harness.key_press(egui::Key::F);
            for action in [Action::Undo, Action::Settings, Action::PhotoInspection] {
                harness.update(|editor| host::App::menu_action(editor, action));
                harness.step();
            }
            harness.read(|editor| {
                assert!(!editor.cropping());
                assert!(editor.menus.panel.is_none() && editor.fullscreen.is_none());
                assert_eq!(editor.document.visit.current_source, selected);
                assert_eq!(editor.document.state.project, before);
            });

            drop(first);
            harness.step();
            assert!(
                harness
                    .get_by_role_and_label(crop, "Crop & straighten")
                    .accesskit_node()
                    .is_disabled()
            );
            drop(second);
            harness.try_run_realtime().unwrap();
            assert!(
                !harness
                    .get_by_role_and_label(crop, "Crop & straighten")
                    .accesskit_node()
                    .is_disabled()
            );
            harness.read(|editor| {
                assert!(!editor.cropping());
                assert!(editor.menus.panel.is_none() && editor.fullscreen.is_none());
            });
            harness
                .get_by_role_and_label(crop, "Crop & straighten")
                .click();
            harness.step();
            harness.read(|editor| assert!(editor.cropping()));
            harness.get_by_label("Cancel").click();
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert!(!editor.cropping());
                assert_eq!(editor.document.state.project, before);
            });
        }
    });
}
