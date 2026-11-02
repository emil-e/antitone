use super::*;

#[test]
fn photo_auto_crop_drawer_updates_only_its_draft_and_dust_can_switch_directly_to_crop() {
    for (size, width) in [
        (egui::vec2(1100., 800.), 315.),
        (egui::vec2(800., 600.), 270.),
    ] {
        setup(|mut h, _directory| {
            set_layout(&mut h, size, width);
            stage(&mut h, "Photo");
            let (id, accepted, normal) = h.read(|editor| {
                (
                    editor.document.visit.current_source.unwrap(),
                    editor.document.state.project.clone(),
                    editor.document.visit.canvas.viewport,
                )
            });
            click_action(&mut h, "Dust repair");
            h.read(|editor| assert!(editor.dusting()));
            click_action(&mut h, "Crop & straighten");
            h.read(|editor| {
                assert!(editor.cropping());
                assert!(!editor.dusting());
            });
            let before = h.read(|editor| editor.document.crop_edit().unwrap().draft);
            let state = h.read(|editor| editor.document.crop_state().unwrap());
            let weak = std::rc::Rc::downgrade(&state);
            drop(state);
            click_action(&mut h, "Auto-crop");
            wait_for(&mut h, |editor| !editor.document.crop_auto_crop_pending());
            let expected = margin::expected_crop(&h, id, h.read(|editor| editor.auto_crop_margin));
            h.read(|editor| {
                let edit = editor.document.crop_edit().unwrap();
                assert_eq!(edit.draft, expected);
                assert_ne!(
                    edit.draft, before,
                    "the drawer action must perform a proposal"
                );
                assert!(edit.auto_problem.is_none(), "{:?}", edit.auto_problem);
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.visit.canvas.viewport, normal);
                assert!(!editor.document.dirty());
            });
            click_action(&mut h, "Reset draft");
            h.read(|editor| {
                assert!(editor.cropping(), "header reset keeps the interaction open");
                assert_eq!(editor.document.crop_edit().unwrap().draft, before);
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.visit.canvas.viewport, normal);
                assert!(!editor.document.dirty());
            });
            h.get_by_label("Cancel").click();
            h.step();
            wait_for(&mut h, |editor| !editor.cropping());
            assert!(
                weak.upgrade().is_none(),
                "surface and completed task release shared draft"
            );
            h.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.visit.canvas.viewport, normal);
            });
            click_action(&mut h, "Crop & straighten");
            click_action(&mut h, "Auto-crop");
            wait_for(&mut h, |editor| !editor.document.crop_auto_crop_pending());
            h.get_by_label("Apply").click();
            h.step();
            wait_for(&mut h, |editor| !editor.cropping());
            h.read(|editor| {
                assert_eq!(editor.document.visit.crop, Some(expected));
                assert_eq!(
                    editor.document.project.source(id).unwrap().crop,
                    Some(expected)
                );
                assert_eq!(
                    editor.history_label(super::super::super::history::Direction::Undo),
                    Some("Crop")
                );
            });
        });
    }
}
