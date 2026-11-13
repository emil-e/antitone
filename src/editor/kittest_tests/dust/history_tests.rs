use super::*;

#[test]
fn undo_redo_keeps_dust_open_and_the_next_pointer_stroke_authors_another_repair() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        let before = h.read(|editor| editor.document.state.project.clone());
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        let repaired = h.read(|editor| editor.document.state.project.clone());
        assert_ne!(before, repaired);
        for (redo, expected) in [(false, &before), (true, &repaired)] {
            h.key_press_modifiers(
                if redo {
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::COMMAND
                },
                egui::Key::Z,
            );
            h.step();
            ready(&mut h, usize::from(redo));
            h.read(|editor| {
                assert_eq!(&editor.document.state.project, expected);
                assert!(editor.dusting());
            });
            h.get_by_label("Active tool: Dust cleanup");
        }
        save_capture(&h.render().unwrap(), "dust-history-retained");
        click(&mut h, egui::pos2(144., 64.), egui::Modifiers::NONE);
        ready(&mut h, 2);
        h.read(|editor| {
            assert!(editor.dusting());
            assert_eq!(editor.history_label(Direction::Undo), Some("Dust mask"));
        });
    });
}
