use super::*;

#[test]
fn crop_omits_shared_navigation_during_its_drag_and_resumes_it_afterward() {
    let mut editor = Editor::state();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.begin_crop();
    let normal = editor.document.visit.canvas.viewport;
    crate::application::test_support::with_application(editor, |app| {
        let context = egui::Context::default();
        presentation::configure_fonts(&context);
        let canvas = egui::Rect::from_min_size(egui::pos2(20., 20.), egui::vec2(400., 300.));
        let render = |events| {
            app.enter(|editor, cx| {
                let mut result = None;
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(500., 400.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let response = ui.interact(
                            canvas,
                            ui.id().with("crop-navigation"),
                            egui::Sense::click_and_drag(),
                        );
                        result = editor.hosted_viewer(ui, &response, None, true, cx);
                    },
                );
                output.textures_delta.clear();
                result.unwrap()
            })
        };
        render(vec![]);
        let before = render(vec![egui::Event::PointerMoved(canvas.center())]);
        app.update(|editor| {
            let mut edit = editor.document.crop_edit_mut().unwrap();
            edit.drag = Some(CropDrag::Move {
                start: canvas.center(),
                crop: edit.draft,
                layout: before,
            });
        });
        let held = render(vec![egui::Event::Zoom(1.5)]);
        assert_eq!(held.scale, before.scale);
        app.update(|editor| editor.document.crop_edit_mut().unwrap().drag = None);
        let released = render(vec![egui::Event::Zoom(1.5)]);
        assert!((released.scale - before.scale * 1.5).abs() < 1e-5);
        app.read(|editor| assert_eq!(editor.document.visit.canvas.viewport, normal));
    });
}
