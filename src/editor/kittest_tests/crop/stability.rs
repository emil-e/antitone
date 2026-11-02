use super::*;

fn painted_frame(h: &Harness<'_, Editor>, canvas: egui::Rect) -> [egui::Pos2; 4] {
    // The crop shade is a ring mesh: its first four vertices bound the unshaded frame.
    // Match its canvas clip and contour, not bracket colours or stroke widths.
    let rings: Vec<_> = h
        .output()
        .shapes
        .iter()
        .filter_map(|shape| {
            if shape.clip_rect == canvas
                && let egui::Shape::Mesh(mesh) = &shape.shape
                && mesh.vertices.len() == 8
                && mesh.indices.len() == 24
            {
                Some(std::array::from_fn(|i| mesh.vertices[i].pos))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        rings.len(),
        1,
        "fixture must expose exactly one crop shade ring"
    );
    rings[0]
}

#[test]
fn moving_crop_keeps_the_painted_frame_fixed_on_motion_hold_and_release_passes() {
    for (density, turns, reflected) in [
        (1., crate::geometry::QuarterTurns::Zero, false),
        (2., crate::geometry::QuarterTurns::One, true),
    ] {
        editor_harness(|mut h, capture| {
            h.input_mut()
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(density);
            h.update(|editor| {
                editor.document.input = crate::processing::InputSettings::Digital;
                editor.sync_document();
            });
            h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
            wait_for(&mut h, |editor| {
                !editor.busy && editor.document.visit.canvas.ready.is_some()
            });
            h.update(|editor| {
                editor.document.source = false;
                editor.document.visit.photo_orientation.turns = turns;
                editor.document.roll_orientation.horizontal = reflected;
                editor.sync_document();
            });
            stage(&mut h, "Photo");
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                .click();
            h.step();
            h.try_run_realtime().unwrap();
            assert_eq!(h.ctx.pixels_per_point(), density);
            h.read(|editor| {
                assert!(
                    editor.cropping(),
                    "Crop must be active before gesture setup"
                )
            });
            let (accepted, normal, draft, source_point) = h.update(|editor| {
                let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
                let draft = Crop {
                    size: [area.width * 3 / 5, area.height * 3 / 5],
                    angle: 0.17,
                    ..Crop::from_region(area)
                }
                .constrained(area)
                .unwrap();
                editor.document.crop_edit_mut().unwrap().draft = draft;
                editor.with_tool_viewport(|viewport| viewport.fit());
                (
                    editor.document.state.project.clone(),
                    editor.document.visit.canvas.viewport,
                    draft,
                    egui::pos2(draft.center[0] as f32, draft.center[1] as f32),
                )
            });
            h.step();
            let initial = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            let frame = [
                initial.image.min,
                initial.image.right_top(),
                initial.image.max,
                initial.image.left_bottom(),
            ];
            let assert_frame = |h: &Harness<'_, Editor>| {
                for (actual, expected) in painted_frame(h, initial.canvas).into_iter().zip(frame) {
                    assert!(
                        actual.distance(expected) * density < 0.01,
                        "crop frame moved on a painted pass: {actual:?} vs {expected:?}"
                    );
                }
            };
            assert_frame(&h);
            let start = initial.image.center();
            h.event(egui::Event::PointerMoved(start));
            h.step();
            h.event(egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            assert_frame(&h);
            let mut end = start;
            for delta in [
                egui::vec2(13.25, 9.5),
                egui::vec2(-17.5, -8.25),
                egui::vec2(7.75, -14.5),
            ] {
                end = start + delta;
                h.event(egui::Event::PointerMoved(end));
                h.step();
                assert_frame(&h); // Inspect the input pass, before any settling repaint.
                h.read(|editor| {
                    assert_ne!(
                        editor.document.crop_edit().unwrap().draft.center,
                        draft.center
                    )
                });
                for _ in 0..2 {
                    h.step();
                    assert_frame(&h);
                }
            }
            let moved = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            assert!(
                moved
                    .screen_point(source_point)
                    .distance(initial.screen_point(source_point))
                    > 1.,
                "the source image must move beneath the stationary frame"
            );
            h.event(egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            for _ in 0..3 {
                h.step();
                assert_frame(&h);
            }
            h.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.visit.canvas.viewport, normal);
            });
            if density == 1. {
                h.update(|editor| editor.document.crop_edit_mut().unwrap().guides = false);
                assert_crop_patch_pixels(&mut h, &capture);
            }
            h.get_by_label("Cancel").click();
            h.step();
            h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        });
    }
}
