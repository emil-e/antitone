use super::*;

#[test]
fn picker_view_and_navigation_are_owned_without_overwriting_normal_preferences() {
    for view in [View::Working, View::Source, View::Native] {
        for source in [false, true] {
            let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
            editor.load_input(InputSettings::Digital);
            let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
            editor.document.roll_orientation.turn_clockwise();
            editor.document.visit.photo_orientation.flips.horizontal = true;
            editor.document.visit.crop = Some(Crop::from_region(Rect {
                x: area.x + 20,
                y: area.y + 10,
                width: area.width - 40,
                height: area.height - 20,
            }));
            editor.document.source = source;
            let canvas = egui::Rect::from_min_size(egui::pos2(20., 30.), egui::vec2(800., 600.));
            let normal = editor.normal_canvas_composition(area, None);
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
            editor.show_picker(display);
            assert!(editor.hosted_picker());
            assert_eq!(editor.document.source, source);
            assert_eq!(editor.canvas_inspection(), view != View::Working);
            let composition = editor.canvas_composition(area, Some(display));
            let layout = editor.with_tool_viewport(|viewport| {
                viewport.layout_composition(canvas, composition, 2.)
            });
            match view {
                View::Working => assert_eq!(composition, normal),
                View::Source => assert_eq!(composition.size(), [area.height, area.width]),
                View::Native => assert_eq!(composition.size(), [area.width, area.height]),
            }
            assert!((layout.source_point(canvas.center()) - anchor).length() < 0.002);
            editor.with_tool_viewport(|viewport| viewport.zoom = Some(2.25));
            if view != View::Working {
                assert_eq!(editor.document.visit.canvas.viewport, viewport);
                editor.document.visit.canvas.last_layout = Some((layout, 2.));
                let next = Display::sample(
                    "Replacement view",
                    if view == View::Source {
                        View::Native
                    } else {
                        View::Source
                    },
                );
                editor.show_picker(next);
                let composition = editor.canvas_composition(area, Some(next));
                let replacement = editor.with_tool_viewport(|viewport| {
                    viewport.layout_composition(canvas, composition, 2.)
                });
                assert_eq!(replacement.scale, previous.scale);
                assert!((replacement.source_point(canvas.center()) - anchor).length() < 0.002);
            }
            editor.end_tool();
            assert!(!editor.hosted_picker());
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
