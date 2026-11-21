use super::*;

#[test]
fn picker_uses_current_composition_and_navigation_without_a_private_view() {
    for source in [false, true] {
        let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
        editor = crate::editor::tests::accepted_input(editor, InputSettings::Digital);
        let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
        editor.document.project.orientation.turn_clockwise();
        editor.photo_orientation_mut().flips.horizontal = true;
        *editor.photo_crop_mut() = Some(Crop::from_region(Rect {
            x: area.x + 20,
            y: area.y + 10,
            width: area.width - 40,
            height: area.height - 20,
        }));
        editor.document.source = source;
        let project = editor.document.project.clone();
        let canvas = egui::Rect::from_min_size(egui::pos2(20., 30.), egui::vec2(800., 600.));
        let normal = editor.normal_canvas_composition(area);
        let viewport = &mut editor.document.visit.canvas.viewport;
        let fit = viewport.layout_composition(canvas, normal, 2.);
        viewport.zoom_at(fit, 2., 2., canvas.center());
        let previous = viewport.layout_composition(canvas, normal, 2.);
        let viewport = *viewport;
        editor.document.visit.canvas.last_layout = Some((previous, 2.));
        let anchor = previous.source_point(canvas.center());
        super::preview(&mut editor, Display::sample("Current view"));
        assert!(editor.test_picker_active());
        assert_eq!(editor.document.source, source);
        assert_eq!(editor.canvas_inspection(), source);
        assert_eq!(editor.canvas_composition(area), normal);
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        let layout =
            editor.with_tool_viewport(|viewport| viewport.layout_composition(canvas, normal, 2.));
        assert!((layout.source_point(canvas.center()) - anchor).length() < 0.002);

        editor.with_tool_viewport(|viewport| viewport.zoom = Some(2.25));
        assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(2.25));
        editor.document.source = !source;
        assert_eq!(editor.canvas_inspection(), !source);
        assert_eq!(editor.canvas_composition(area), normal);
        let navigated = editor.document.visit.canvas.viewport;
        super::preview(&mut editor, Display::sample("Replacement interaction"));
        assert!(editor.test_picker_active());
        assert_eq!(editor.document.visit.canvas.viewport, navigated);
        assert_eq!(editor.canvas_inspection(), !source);
        editor.end_tool();
        assert!(!editor.test_picker_active());
        assert_eq!(editor.document.source, !source);
        assert_eq!(editor.document.visit.canvas.viewport, navigated);
        assert_eq!(editor.document.project, project);
    }
}
