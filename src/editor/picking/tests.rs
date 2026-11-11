use super::*;

#[test]
fn picker_view_and_navigation_are_owned_without_overwriting_normal_preferences() {
    for view in [View::Working, View::Source] {
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
            let canvas = egui::Rect::from_min_size(egui::pos2(20., 30.), egui::vec2(800., 600.));
            let normal = editor.normal_canvas_composition(area);
            let fit = editor
                .document
                .visit
                .canvas
                .viewport
                .layout_composition(canvas, normal, 2.);
            editor
                .document
                .visit
                .canvas
                .viewport
                .zoom_at(fit, 2., 2., canvas.center());
            let previous = editor
                .document
                .visit
                .canvas
                .viewport
                .layout_composition(canvas, normal, 2.);
            editor.document.visit.canvas.last_layout = Some((previous, 2.));
            let viewport = editor.document.visit.canvas.viewport;
            let anchor = previous.source_point(canvas.center());
            let display = Display::sample("View ownership", view);
            super::preview(&mut editor, display);
            assert!(editor.test_picker_active());
            assert_eq!(editor.document.source, source);
            assert_eq!(editor.canvas_inspection(), view != View::Working);
            let composition = editor.canvas_composition(area);
            let layout = editor.with_tool_viewport(|viewport| {
                viewport.layout_composition(canvas, composition, 2.)
            });
            match view {
                View::Working => assert_eq!(composition, normal),
                View::Source => assert_eq!(composition.size(), [area.height, area.width]),
            }
            assert!((layout.source_point(canvas.center()) - anchor).length() < 0.002);
            editor.with_tool_viewport(|viewport| viewport.zoom = Some(2.25));
            if view != View::Working {
                assert_eq!(editor.document.visit.canvas.viewport, viewport);
                editor.document.visit.canvas.last_layout = Some((layout, 2.));
                let next = Display::sample("Replacement view", View::Source);
                super::preview(&mut editor, next);
                let composition = editor.canvas_composition(area);
                let replacement = editor.with_tool_viewport(|viewport| {
                    viewport.layout_composition(canvas, composition, 2.)
                });
                assert_eq!(replacement.scale, previous.scale);
                assert!((replacement.source_point(canvas.center()) - anchor).length() < 0.002);
            }
            editor.end_tool();
            assert!(!editor.test_picker_active());
            assert_eq!(editor.document.source, source);
            assert_eq!(editor.canvas_inspection(), source);
            if view == View::Working {
                assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(2.25));
            } else {
                assert_eq!(editor.document.visit.canvas.viewport, viewport);
            }
        }
    }
}
