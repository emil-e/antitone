use super::*;
use crate::geometry::Flips;
use std::{
    any::Any,
    cell::{Ref, RefMut},
};

impl DocumentScope {
    pub(in crate::editor) fn crop_edit(&self) -> Option<Ref<'_, CropEdit>> {
        let tool = self.visit.hosted_tool.installed()?;
        let crop = (tool as &dyn Any).downcast_ref::<crop_editor::CropTool>()?;
        Some(crop.edit.as_ref()?.borrow())
    }
    pub(in crate::editor) fn crop_edit_mut(&mut self) -> Option<RefMut<'_, CropEdit>> {
        let tool = self.visit.hosted_tool.installed()?;
        let crop = (tool as &dyn Any).downcast_ref::<crop_editor::CropTool>()?;
        Some(crop.edit.as_ref()?.borrow_mut())
    }
    pub(in crate::editor) fn set_crop_edit(&mut self, edit: Option<CropEdit>) {
        if let Some(edit) = edit {
            self.tool.clear();
            self.hosted_tool = tool::Slot::Empty;
            self.visit.hosted_tool =
                tool::Slot::Installed(Box::new(crop_editor::CropTool::from_edit(edit)));
        } else {
            self.clear_crop_edit();
        }
    }
    pub(in crate::editor) fn crop_state(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<CropEdit>>> {
        let tool = self.visit.hosted_tool.installed()?;
        (tool as &dyn Any)
            .downcast_ref::<crop_editor::CropTool>()?
            .edit
            .clone()
    }
    pub(in crate::editor) fn crop_auto_crop_pending(&self) -> bool {
        self.visit
            .hosted_tool
            .installed()
            .and_then(|tool| (tool as &dyn Any).downcast_ref::<crop_editor::CropTool>())
            .is_some_and(crop_editor::CropTool::auto_crop_pending)
    }
    pub(in crate::editor) fn crop_viewport_mut(&mut self) -> Option<&mut Viewport> {
        let tool = self.visit.hosted_tool.installed_mut()?;
        Some(
            &mut (tool as &mut dyn Any)
                .downcast_mut::<crop_editor::CropTool>()?
                .viewport,
        )
    }
}

impl CropTool {
    fn auto_crop_pending(&self) -> bool {
        self.auto_crop_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    fn from_edit(edit: CropEdit) -> Self {
        Self {
            edit: Some(Rc::new(RefCell::new(edit))),
            ..Default::default()
        }
    }
}

pub(in crate::editor) fn request_auto_crop(
    photo: SourceId,
    editor: &mut Editor,
    cx: &UiContext<'_, Editor>,
) {
    editor.with_current_tool(|tool, editor| {
        let crop = (tool as &mut dyn std::any::Any)
            .downcast_mut::<CropTool>()
            .unwrap();
        crop.request_auto_crop(editor, photo, cx);
    });
}

const AREA: Rect = Rect {
    x: 12,
    y: 7,
    width: 120,
    height: 90,
};
const PATCH: Rect = Rect {
    x: 30,
    y: 20,
    width: 60,
    height: 40,
};

#[test]
fn painting_uses_the_pass_frame_even_after_a_different_draft_has_been_staged() {
    fn paint_pass(h: &mut Harness, layout: CanvasLayout) -> [egui::Pos2; 4] {
        let mut output = h.context.run_ui(
            egui::RawInput {
                screen_rect: Some(layout.canvas),
                ..Default::default()
            },
            |ui| {
                h.editor.with_current_tool(|tool, _| {
                    (tool as &mut dyn std::any::Any)
                        .downcast_mut::<CropTool>()
                        .unwrap()
                        .paint(ui, layout);
                });
            },
        );
        output.textures_delta.clear();
        let rings: Vec<[egui::Pos2; 4]> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Mesh(mesh) = &shape.shape
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
            "fixture must discover the crop shade contour"
        );
        rings[0]
    }
    let corners = |layout: CanvasLayout| {
        [
            layout.image.min,
            layout.image.right_top(),
            layout.image.max,
            layout.image.left_bottom(),
        ]
    };
    let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
    let pass = h.frame(vec![]);
    let next = Crop {
        center: [73., 51.],
        size: [40, 24],
        angle: 0.17,
    };
    next.coverage().unwrap().bounds(AREA).unwrap();
    h.editor
        .document
        .crop_edit_mut()
        .unwrap()
        .install_proposal(next);
    assert_eq!(paint_pass(&mut h, pass), corners(pass));
    let next_pass = h.frame(vec![]);
    assert_ne!(
        next_pass.image, pass.image,
        "the next pass must adopt the staged frame"
    );
    assert_eq!(paint_pass(&mut h, next_pass), corners(next_pass));
}

#[test]
fn configuration_seeds_the_draft_once_without_borrowing_normal_navigation_as_its_own() {
    fn query(
        tool: &mut CropTool,
        viewport: &mut Viewport,
        composition: Option<Composition>,
    ) -> (Option<Composition>, Viewport) {
        let mut view = tool::ViewConfig {
            viewport,
            composition,
            source_outside_frame: false,
            inset: 0.,
            constrain_pan: true,
            fit_once: None,
            primary_interaction: false,
            navigation_enabled: true,
        };
        tool::Tool::configure(tool, &mut view);
        (view.composition, *view.viewport)
    }
    let base = Composition::with_crop(
        AREA,
        RollOrientation {
            horizontal: true,
            ..Default::default()
        },
        PhotoOrientation {
            turns: crate::geometry::QuarterTurns::One,
            ..Default::default()
        },
        Some(Crop::from_region(PATCH)),
    )
    .unwrap();
    let mut normal = Viewport::default();
    normal.zoom = Some(1.75);
    let before = normal;
    let mut tool = CropTool::default();
    assert_eq!(query(&mut tool, &mut normal, None), (None, before));
    assert!(tool.edit.is_none());
    assert_eq!(query(&mut tool, &mut normal, Some(base)).0, Some(base));
    let changed = Crop {
        size: [42, 28],
        ..base.crop()
    };
    tool.edit.as_ref().unwrap().borrow_mut().draft = changed;
    tool.viewport.zoom = Some(2.5);
    for _ in 0..3 {
        let (composition, selected) = query(&mut tool, &mut normal, Some(base));
        assert_eq!(composition.unwrap().crop(), changed);
        assert_eq!(selected.zoom, Some(2.5));
        assert_eq!(normal, before);
    }
}

#[test]
fn photo_scope_drop_discards_crop_navigation_without_copying_it_into_the_next_visit() {
    let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
    h.editor.document.visit.canvas.viewport.zoom = Some(3.5);
    let normal = h.editor.document.visit.canvas.viewport;
    h.frame(vec![]);
    assert_ne!(*h.editor.document.crop_viewport_mut().unwrap(), normal);
    let project = h.editor.document.state.project.clone();
    let status = h.editor.status.clone();
    h.editor.replace_photo_scope(1);
    assert!(h.editor.document.crop_edit().is_none());
    assert_eq!(h.editor.document.visit.canvas.viewport, normal);
    assert!(h.editor.document.visit.crop.is_none());
    assert_eq!(h.editor.document.state.project, project);
    assert_eq!(
        h.editor.status, status,
        "owner retirement must not simulate Dismiss"
    );
}

#[test]
fn outside_drag_straightens_about_crop_centre_through_reflections_and_turns() {
    for turns in [
        crate::geometry::QuarterTurns::Zero,
        crate::geometry::QuarterTurns::One,
        crate::geometry::QuarterTurns::Two,
        crate::geometry::QuarterTurns::Three,
    ] {
        for horizontal in [false, true] {
            let mut h = Harness::new(
                turns,
                RollOrientation {
                    horizontal,
                    ..Default::default()
                },
                None,
            );
            let before = h.frame(vec![]);
            let start = before.image.right_center() + egui::vec2(12., 0.);
            let radius = start.x - before.image.center().x;
            let delta = 0.17_f32;
            let end = before.image.center() + egui::vec2(delta.cos(), delta.sin()) * radius;
            h.press(start, true);
            let after = h.frame(vec![egui::Event::PointerMoved(end)]);
            assert_eq!(h.cursor, egui::CursorIcon::Crosshair);
            let draft = h.editor.document.crop_edit().unwrap().draft;
            assert!(
                (draft.angle + f64::from(delta) * before.composition.handedness()).abs() < 2e-7
            );
            assert_eq!(draft.center, Crop::from_region(PATCH).center);
            assert_eq!(draft.size, [PATCH.width, PATCH.height]);
            assert_eq!(after.image, before.image);
            draft.coverage().unwrap().bounds(AREA).unwrap();
            assert!(h.editor.document.visit.crop.is_none());
            h.press(end, false);
            assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
        }
    }
}

#[test]
fn rotated_resize_stays_frame_aligned_and_move_keeps_coverage_inside_source() {
    let mut h = Harness::new(
        crate::geometry::QuarterTurns::One,
        RollOrientation::default(),
        None,
    );
    h.editor.document.crop_edit_mut().unwrap().draft =
        Crop::from_region(PATCH).straightened(AREA, 0.31).unwrap();
    let before = h.frame(vec![]);
    assert!(
        before
            .canvas
            .shrink(19.)
            .contains_rect(before.screen_region(AREA))
    );
    let marker = egui::pos2((AREA.x + 17) as f32, (AREA.y + 23) as f32);
    let original = before.screen_point(marker);
    let start = before.image.max;
    let end = start - egui::vec2(40., 30.);
    h.press(start, true);
    let after = h.frame(vec![egui::Event::PointerMoved(end)]);
    assert!(after.image.min.distance(before.image.min) < 0.001);
    assert!(after.composition.size()[0] < before.composition.size()[0]);
    assert!(after.composition.size()[1] < before.composition.size()[1]);
    assert_eq!(
        after.composition.crop().angle,
        before.composition.crop().angle
    );
    let draft = h.editor.document.crop_edit().unwrap().draft;
    draft.coverage().unwrap().bounds(AREA).unwrap();
    h.press(end, false);
    let layout = h.frame(vec![]);
    assert_eq!(layout.scale, before.scale);
    assert!(layout.image.min.distance(after.image.min) < 0.001);
    assert!(layout.image.max.distance(after.image.max) < 0.001);
    assert!(layout.screen_point(marker).distance(original) < 0.001);
    h.press(layout.image.center(), true);
    h.frame(vec![egui::Event::PointerMoved(egui::pos2(-1000., -1000.))]);
    h.editor
        .document
        .crop_edit()
        .unwrap()
        .draft
        .coverage()
        .unwrap()
        .bounds(AREA)
        .unwrap();
}

#[test]
fn full_source_is_visible_and_resize_release_does_not_zoom_or_move_the_image() {
    for turns in [
        crate::geometry::QuarterTurns::Zero,
        crate::geometry::QuarterTurns::One,
        crate::geometry::QuarterTurns::Two,
        crate::geometry::QuarterTurns::Three,
    ] {
        let mut h = Harness::new(turns, RollOrientation::default(), None);
        let before = h.frame(vec![]);
        let source_bounds = before.screen_region(AREA);
        assert!(before.canvas.shrink(19.).contains_rect(source_bounds));
        let marker = egui::pos2((AREA.x + 17) as f32, (AREA.y + 23) as f32);
        let original = before.screen_point(marker);
        let start = before.image.max;
        let end = start - egui::vec2(30., 25.);
        h.press(start, true);
        let smaller = h.frame(vec![egui::Event::PointerMoved(end)]);
        assert_eq!(smaller.scale, before.scale);
        assert!(smaller.screen_point(marker).distance(original) < 0.001);
        h.press(end, false);
        let released = h.frame(vec![]);
        assert_eq!(released.scale, before.scale);
        assert!(released.screen_point(marker).distance(original) < 0.001);
        assert!(released.image.max.distance(smaller.image.max) < 0.001);
        // One edge drag can reach the available source boundary, without intermediate refits.
        let end = released.screen_region(AREA).max;
        assert!(released.canvas.contains(end));
        h.press(released.image.max, true);
        let larger = h.frame(vec![egui::Event::PointerMoved(end)]);
        h.press(end, false);
        let finished = h.frame(vec![]);
        assert!(larger.composition.size()[0] > released.composition.size()[0]);
        assert!(larger.composition.size()[1] > released.composition.size()[1]);
        assert_eq!(finished.scale, before.scale);
        assert!(finished.screen_point(marker).distance(original) < 0.001);
        assert!(finished.image.max.distance(end) < before.scale * 1.01);
    }
}

#[test]
fn angle_control_reframing_keeps_magnification_and_screen_pivot() {
    let mut h = Harness::new(
        crate::geometry::QuarterTurns::One,
        RollOrientation::default(),
        None,
    );
    let before = h.frame(vec![]);
    let crop = before.composition.crop().straightened(AREA, 0.23).unwrap();
    let next = Composition::with_crop(
        AREA,
        h.editor.document.roll_orientation,
        h.editor.document.visit.photo_orientation,
        Some(crop),
    )
    .unwrap();
    reframe_crop(
        h.editor.document.crop_viewport_mut().unwrap(),
        before,
        next,
        1.,
        true,
    );
    h.editor.document.crop_edit_mut().unwrap().draft = crop;
    let after = h.frame(vec![]);
    assert_eq!(before.scale, after.scale);
    assert!(before.image.center().distance(after.image.center()) < 0.001);
    assert!(
        after
            .screen_point(egui::pos2(crop.center[0] as f32, crop.center[1] as f32))
            .distance(before.image.center())
            < 0.001
    );
    crop.coverage().unwrap().bounds(AREA).unwrap();
}

#[test]
fn crop_brackets_and_thirds_follow_the_frame_without_changing_geometry() {
    let mut h = Harness::new(
        crate::geometry::QuarterTurns::Zero,
        RollOrientation::default(),
        None,
    );
    let layout = h.frame(vec![]);
    let thirds: Vec<_> = h
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.width == 0.5 && stroke.color == egui::Color32::WHITE =>
            {
                Some(*points)
            }
            _ => None,
        })
        .collect();
    assert_eq!(thirds.len(), 4);
    assert_eq!(
        thirds[0],
        [
            egui::pos2(
                egui::lerp(layout.image.x_range(), 1. / 3.),
                layout.image.top()
            ),
            egui::pos2(
                egui::lerp(layout.image.x_range(), 1. / 3.),
                layout.image.bottom()
            )
        ]
    );
    assert!(
        h.shapes.iter().all(
            |s| !matches!(&s.shape,egui::Shape::Rect(rect) if rect.fill==egui::Color32::WHITE)
        )
    );
    let brackets = h
        .shapes
        .iter()
        .filter(|shape| match &shape.shape {
            egui::Shape::LineSegment { stroke, .. } => {
                stroke.width == 2.5 && stroke.color == egui::Color32::WHITE
            }
            _ => false,
        })
        .count();
    assert_eq!(brackets, 12);
    h.press(layout.image.max, true);
    let resized = h.frame(vec![egui::Event::PointerMoved(
        layout.image.max - egui::vec2(40., 30.),
    )]);
    let first = h
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.width == 0.5 && stroke.color == egui::Color32::WHITE =>
            {
                Some(*points)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        first,
        [
            egui::pos2(
                egui::lerp(resized.image.x_range(), 1. / 3.),
                resized.image.top()
            ),
            egui::pos2(
                egui::lerp(resized.image.x_range(), 1. / 3.),
                resized.image.bottom()
            )
        ]
    );
    h.press(resized.image.max, false);
    let draft = h.editor.document.crop_edit().unwrap().draft;
    h.editor.document.crop_edit_mut().unwrap().guides = false;
    let no_guides = h.frame(vec![]);
    assert_eq!(no_guides.composition, resized.composition);
    assert_eq!(h.editor.document.crop_edit().unwrap().draft, draft);
    assert!(!h.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::LineSegment { stroke, .. } => {
            stroke.width == 0.5 && stroke.color == egui::Color32::WHITE
        }
        _ => false,
    }));
}

fn assert_rotation_cursor(h: &Harness, point: egui::Pos2, pivot: egui::Pos2) {
    let expected = icons::rotation_cursor(
        &h.context,
        egui::Id::new("expected-rotation-cursor"),
        (point - pivot).angle() + std::f32::consts::FRAC_PI_2,
    );
    assert_eq!(h.cursor_image.as_ref(), Some(&expected));
}

#[test]
fn rotation_hover_follows_edges_and_corners_without_changing_geometry() {
    let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
    let before = h.frame(vec![]);
    let draft = h.editor.document.crop_edit().unwrap().draft;
    let cursors = [
        egui::CursorIcon::ResizeNwSe,
        egui::CursorIcon::ResizeNeSw,
        egui::CursorIcon::ResizeNwSe,
        egui::CursorIcon::ResizeNeSw,
        egui::CursorIcon::ResizeHorizontal,
        egui::CursorIcon::ResizeHorizontal,
        egui::CursorIcon::ResizeVertical,
        egui::CursorIcon::ResizeVertical,
    ];
    for ((point, _), cursor) in handles(before.image).into_iter().zip(cursors) {
        h.frame(vec![egui::Event::PointerMoved(point)]);
        assert_eq!(h.cursor, cursor);
        assert!(h.cursor_image.is_none());
    }
    h.frame(vec![egui::Event::PointerMoved(before.image.center())]);
    assert_eq!(h.cursor, egui::CursorIcon::Grab);
    assert!(h.cursor_image.is_none());
    let centre = before.image.center();
    for direction in [
        egui::vec2(1., 0.),
        egui::vec2(0., 1.),
        egui::vec2(-1., 0.),
        egui::vec2(0., -1.),
        egui::vec2(1., 1.),
        egui::vec2(-1., 1.),
        egui::vec2(-1., -1.),
        egui::vec2(1., -1.),
    ] {
        let point = centre + direction * (before.image.size() * 0.5 + egui::Vec2::splat(18.));
        let after = h.frame(vec![egui::Event::PointerMoved(point)]);
        assert_eq!(h.cursor, egui::CursorIcon::Crosshair); // Unsupported-backend fallback only.
        assert_rotation_cursor(&h, point, centre);
        assert_eq!(after.image, before.image);
        assert_eq!(after.composition, before.composition);
        assert_eq!(after.scale, before.scale);
        assert_eq!(h.editor.document.crop_edit().unwrap().draft, draft);
        assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
        assert!(h.editor.document.visit.crop.is_none());
    }
    // Nearby bearings rotate smoothly instead of using edge/quadrant buckets.
    let point = before.image.right_center() + egui::vec2(18., 3.);
    h.frame(vec![egui::Event::PointerMoved(point)]);
    assert_rotation_cursor(&h, point, centre);
}

#[test]
fn rotation_drag_uses_original_pivot_inside_the_frame_and_releases_to_move_hover() {
    for turns in [
        crate::geometry::QuarterTurns::Zero,
        crate::geometry::QuarterTurns::One,
        crate::geometry::QuarterTurns::Two,
        crate::geometry::QuarterTurns::Three,
    ] {
        for horizontal in [false, true] {
            let mut h = Harness::new(
                turns,
                RollOrientation {
                    horizontal,
                    ..Default::default()
                },
                None,
            );
            let before = h.frame(vec![]);
            let start = before.image.right_center() + egui::vec2(18., 0.);
            let end = before.image.center() + egui::vec2(40., 15.);
            assert!(before.image.contains(end));
            h.press(start, true);
            let after = h.frame(vec![egui::Event::PointerMoved(end)]);
            assert_rotation_cursor(&h, end, before.image.center());
            assert!(matches!(
                h.editor.document.crop_edit().unwrap().drag,
                Some(CropDrag::Straighten { .. })
            ));
            let draft = h.editor.document.crop_edit().unwrap().draft;
            let turn = f64::from(15_f32.atan2(40.));
            assert!((draft.angle + turn * before.composition.handedness()).abs() < 2e-7);
            assert_eq!(draft.center, Crop::from_region(PATCH).center);
            assert_eq!(after.image.center(), before.image.center());
            assert!(h.editor.document.visit.crop.is_none());
            h.frame(vec![egui::Event::PointerMoved(
                before.canvas.max + egui::vec2(30., 30.),
            )]);
            assert_eq!(h.cursor, egui::CursorIcon::Crosshair);
            assert!(h.cursor_image.is_none());
            h.frame(vec![egui::Event::PointerMoved(end)]);
            assert_rotation_cursor(&h, end, before.image.center());
            assert_eq!(h.editor.document.crop_edit().unwrap().draft, draft);
            h.press(end, false);
            assert_eq!(h.cursor, egui::CursorIcon::Grab);
            assert!(h.cursor_image.is_none());
            h.frame(vec![]);
            assert_eq!(h.cursor, egui::CursorIcon::Grab);
            assert!(h.cursor_image.is_none());
            assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
            assert_eq!(h.editor.document.crop_edit().unwrap().draft, draft);
        }
    }
}

#[test]
fn straightening_release_on_a_handle_restores_the_native_resize_cursor() {
    let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
    let before = h.frame(vec![]);
    let end = before.image.right_center();
    h.press(end + egui::vec2(18., 0.), true);
    h.frame(vec![egui::Event::PointerMoved(end)]);
    assert_rotation_cursor(&h, end, before.image.center());
    h.press(end, false);
    assert_eq!(h.cursor, egui::CursorIcon::ResizeHorizontal);
    assert!(h.cursor_image.is_none());
    assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
}

#[test]
fn move_and_resize_drags_keep_their_native_cursors_when_crossing_the_frame() {
    for resizing in [false, true] {
        let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
        let before = h.frame(vec![]);
        let start = if resizing {
            before.image.max
        } else {
            before.image.center()
        };
        let end = before.image.max + egui::vec2(18., 18.);
        h.press(start, true);
        h.frame(vec![egui::Event::PointerMoved(end)]);
        assert_eq!(
            h.cursor,
            if resizing {
                egui::CursorIcon::ResizeNwSe
            } else {
                egui::CursorIcon::Grabbing
            }
        );
        assert!(h.cursor_image.is_none());
        assert!(matches!(
            h.editor.document.crop_edit().unwrap().drag,
            Some(CropDrag::Move { .. } | CropDrag::Resize { .. })
        ));
        let released = h.press(end, false);
        let point = released.image.right_center() + egui::vec2(18., 0.);
        h.frame(vec![egui::Event::PointerMoved(point)]);
        assert_rotation_cursor(&h, point, released.image.center());
    }
}

#[test]
fn release_ends_the_owned_crop_drag_even_when_canvas_input_becomes_disabled() {
    let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
    let layout = h.frame(vec![]);
    h.press(layout.image.max, true);
    let end = layout.image.max - egui::vec2(25., 18.);
    h.frame(vec![egui::Event::PointerMoved(end)]);
    assert!(h.editor.document.crop_edit().unwrap().drag.is_some());
    assert!(
        !h.editor
            .with_tool_view(None, |view| view.navigation_enabled)
    );
    h.enabled = false;
    h.press(end, false);
    assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
    assert!(
        h.editor
            .with_tool_view(None, |view| view.navigation_enabled)
    );
    h.enabled = true;
    let layout = h.frame(vec![]);
    h.press(layout.image.center(), true);
    h.frame(vec![egui::Event::PointerMoved(
        layout.image.center() + egui::vec2(10., 8.),
    )]);
    assert!(matches!(
        h.editor.document.crop_edit().unwrap().drag,
        Some(CropDrag::Move { .. })
    ));
}

#[test]
fn rotation_cursor_respects_disabled_popup_and_canvas_exit_at_both_scales() {
    for scale in [1., 2.] {
        let mut h = Harness::new(Default::default(), RollOrientation::default(), None);
        h.context.set_pixels_per_point(scale);
        // Settle egui's pending zoom before fitting the crop to the viewport.
        let mut warmup = h.context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600., 400.),
                )),
                ..Default::default()
            },
            |_| {},
        );
        warmup.textures_delta.clear();
        let before = h.frame(vec![]);
        let draft = h.editor.document.crop_edit().unwrap().draft;
        for point in [
            before.canvas.min + egui::vec2(2., 2.),
            before.canvas.max - egui::vec2(2., 2.),
        ] {
            h.frame(vec![egui::Event::PointerMoved(point)]);
            assert_rotation_cursor(&h, point, before.image.center());
        }
        let point = before.image.right_center() + egui::vec2(18., 0.);
        h.enabled = false;
        h.frame(vec![egui::Event::PointerMoved(point)]);
        h.press(point, true);
        h.frame(vec![egui::Event::PointerMoved(
            point + egui::vec2(10., 10.),
        )]);
        assert_eq!(h.cursor, egui::CursorIcon::Default);
        assert!(h.cursor_image.is_none());
        assert_eq!(h.editor.document.crop_edit().unwrap().draft, draft);
        assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
        h.press(point, false);
        h.enabled = true;
        egui::Popup::open_id(&h.context, egui::Id::new("crop-input-owner"));
        h.frame(vec![egui::Event::PointerMoved(point)]);
        assert!(h.cursor_image.is_none());
        assert_eq!(h.cursor, egui::CursorIcon::Default);
        egui::Popup::close_all(&h.context);
        h.frame(vec![egui::Event::PointerMoved(point)]);
        assert_rotation_cursor(&h, point, before.image.center());
        h.frame(vec![egui::Event::PointerMoved(
            before.canvas.max + egui::vec2(2., 2.),
        )]);
        assert!(h.cursor_image.is_none());
        h.frame(vec![egui::Event::PointerGone]);
        assert!(h.cursor_image.is_none());
        h.frame(vec![egui::Event::PointerMoved(point)]);
        assert_rotation_cursor(&h, point, before.image.center());
        assert_eq!(h.editor.document.crop_edit().unwrap().draft, draft);
    }
}

struct Harness {
    editor: Editor,
    enabled: bool,
    context: egui::Context,
    cursor: egui::CursorIcon,
    cursor_image: Option<egui::CustomCursorImage>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Harness {
    fn new(
        turns: crate::geometry::QuarterTurns,
        roll: RollOrientation,
        ratio: Option<f64>,
    ) -> Self {
        let mut editor = Editor::state();
        editor.document.roll_orientation = roll;
        editor.document.visit.photo_orientation = PhotoOrientation {
            turns,
            flips: Flips {
                horizontal: true,
                vertical: false,
            },
        };
        editor.document.set_crop_edit(Some(CropEdit {
            draft: Crop::from_region(PATCH),
            base: Composition::new(AREA, roll, editor.document.visit.photo_orientation).unwrap(),
            ratio,
            drag: None,
            layout: None,
            guides: true,
            auto_problem: None,
            refit: false,
            reveal_controls: false,
        }));
        Self {
            editor,
            enabled: true,
            context: egui::Context::default(),
            cursor: egui::CursorIcon::Default,
            cursor_image: None,
            shapes: Vec::new(),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> CanvasLayout {
        let mut result = None;
        let mut output = self.context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600., 400.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        if !self.enabled {
                            ui.disable();
                        }
                        let (canvas, response) = ui.allocate_exact_size(
                            ui.available_size(),
                            egui::Sense::click_and_drag(),
                        );
                        let composition = Composition::with_crop(
                            AREA,
                            self.editor.document.roll_orientation,
                            self.editor.document.visit.photo_orientation,
                            Some(self.editor.document.crop_edit().unwrap().draft),
                        )
                        .unwrap();
                        let mut layout = self
                            .editor
                            .document
                            .crop_viewport_mut()
                            .unwrap()
                            .layout_crop(
                                canvas.shrink(20.),
                                composition,
                                ui.ctx().pixels_per_point(),
                            );
                        layout.canvas = canvas;
                        let input_allowed = self.editor.canvas_input_allowed(ui, canvas);
                        self.editor.with_current_tool(|tool, editor| {
                            let tool = (tool as &mut dyn std::any::Any)
                                .downcast_mut::<CropTool>()
                                .unwrap();
                            let layout = ui
                                .add_enabled_ui(input_allowed, |ui| {
                                    tool.interact(ui, &response, layout, &mut editor.status)
                                })
                                .inner;
                            tool.paint(ui, layout);
                            if input_allowed {
                                tool.rotation_cursor(ui, &response, layout);
                            }
                            result = Some(layout);
                        });
                    });
            },
        );
        self.cursor = output.platform_output.cursor_icon;
        self.cursor_image = output.platform_output.cursor_image.take();
        self.shapes = std::mem::take(&mut output.shapes);
        output.textures_delta.clear();
        result.unwrap()
    }

    fn press(&mut self, point: egui::Pos2, pressed: bool) -> CanvasLayout {
        self.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ])
    }
}

#[test]
fn dragging_image_moves_content_not_the_frame_through_exact_orientations() {
    for turns in [
        crate::geometry::QuarterTurns::Zero,
        crate::geometry::QuarterTurns::One,
        crate::geometry::QuarterTurns::Two,
        crate::geometry::QuarterTurns::Three,
    ] {
        for horizontal in [false, true] {
            for vertical in [false, true] {
                let mut h = Harness::new(
                    turns,
                    RollOrientation {
                        horizontal,
                        vertical,
                        turns: crate::geometry::QuarterTurns::One,
                    },
                    None,
                );
                let before = h.frame(vec![]);
                let start = before.image.center();
                let end = start + egui::vec2(31., 24.);
                h.press(start, true);
                let after = h.frame(vec![egui::Event::PointerMoved(end)]);
                assert_eq!(after.image, before.image);
                assert_eq!(after.composition.size(), before.composition.size());
                let delta = before.source_point(end) - before.source_point(start);
                let wanted = Crop {
                    center: [
                        Crop::from_region(PATCH).center[0] - f64::from(delta.x),
                        Crop::from_region(PATCH).center[1] - f64::from(delta.y),
                    ],
                    size: [PATCH.width, PATCH.height],
                    angle: 0.,
                }
                .constrained(AREA)
                .unwrap();
                assert_eq!(h.editor.document.crop_edit().unwrap().draft, wanted);
                assert!(h.editor.document.visit.crop.is_none());
                h.press(end, false);
            }
        }
    }
}

#[test]
fn every_edge_and_corner_resizes_without_moving_its_opposite_anchor() {
    for turns in [
        crate::geometry::QuarterTurns::Zero,
        crate::geometry::QuarterTurns::One,
        crate::geometry::QuarterTurns::Two,
        crate::geometry::QuarterTurns::Three,
    ] {
        for horizontal in [false, true] {
            for vertical in [false, true] {
                for index in 0..8 {
                    let mut h = Harness::new(
                        turns,
                        RollOrientation {
                            horizontal,
                            vertical,
                            turns: crate::geometry::QuarterTurns::Three,
                        },
                        None,
                    );
                    let before = h.frame(vec![]);
                    let (start, handle) = handles(before.image)[index];
                    let fixed = handles(before.image)
                        .into_iter()
                        .find(|(_, other)| *other == handle.map(|axis| -axis))
                        .unwrap()
                        .0;
                    h.press(start, true);
                    let end =
                        start - egui::vec2(f32::from(handle[0]) * 23., f32::from(handle[1]) * 17.);
                    let after = h.frame(vec![egui::Event::PointerMoved(end)]);
                    let cursor = [
                        egui::CursorIcon::ResizeNwSe,
                        egui::CursorIcon::ResizeNeSw,
                        egui::CursorIcon::ResizeNwSe,
                        egui::CursorIcon::ResizeNeSw,
                        egui::CursorIcon::ResizeHorizontal,
                        egui::CursorIcon::ResizeHorizontal,
                        egui::CursorIcon::ResizeVertical,
                        egui::CursorIcon::ResizeVertical,
                    ][index];
                    assert_eq!(h.cursor, cursor);
                    let new_fixed = handles(after.image)
                        .into_iter()
                        .find(|(_, other)| *other == handle.map(|axis| -axis))
                        .unwrap()
                        .0;
                    assert!(
                        new_fixed.distance(fixed) <= before.scale * 1.01,
                        "{turns:?} {handle:?}: {fixed:?} -> {new_fixed:?}"
                    );
                    assert!(AREA.contains_region(after.area));
                    assert_ne!(after.area, PATCH);
                    assert!(h.editor.document.visit.crop.is_none());
                    h.press(end, false);
                    assert!(h.editor.document.crop_edit().unwrap().drag.is_none());
                    let released = h.frame(vec![]);
                    assert!((released.scale - before.scale).abs() < 1e-6);
                    assert!(released.image.min.distance(after.image.min) < 0.001);
                    assert!(released.image.max.distance(after.image.max) < 0.001);
                }
            }
        }
    }
}

#[test]
fn aspect_locked_edges_preserve_proportions_to_integer_pixel_precision() {
    for index in 0..8 {
        let mut h = Harness::new(
            crate::geometry::QuarterTurns::Zero,
            RollOrientation::default(),
            Some(1.5),
        );
        let before = h.frame(vec![]);
        let (start, handle) = handles(before.image)[index];
        h.press(start, true);
        let end = start - egui::vec2(f32::from(handle[0]) * 90., f32::from(handle[1]) * 60.);
        let after = h.frame(vec![egui::Event::PointerMoved(end)]);
        let [width, height] = after.composition.size();
        assert!((f64::from(width) - 1.5 * f64::from(height)).abs() <= 1.5);
        assert!(AREA.contains_region(after.area));
    }
}
