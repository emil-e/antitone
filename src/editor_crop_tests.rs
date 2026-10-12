use super::*;
use crate::geometry::Flips;
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
            let draft = h.editor.crop_edit.as_ref().unwrap().draft;
            assert!(
                (draft.angle + f64::from(delta) * before.composition.handedness()).abs() < 2e-7
            );
            assert_eq!(draft.center, Crop::from_region(PATCH).center);
            assert_eq!(draft.size, [PATCH.width, PATCH.height]);
            assert_eq!(after.image, before.image);
            draft.coverage().unwrap().bounds(AREA).unwrap();
            assert!(h.editor.crop.is_none());
            h.press(end, false);
            assert!(h.editor.crop_edit.as_ref().unwrap().drag.is_none());
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
    h.editor.crop_edit.as_mut().unwrap().draft =
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
    let draft = h.editor.crop_edit.as_ref().unwrap().draft;
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
        .crop_edit
        .as_ref()
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
        h.editor.roll_orientation,
        h.editor.photo_orientation,
        Some(crop),
    )
    .unwrap();
    reframe_crop(&mut h.editor.viewport, before, next, 1., true);
    h.editor.crop_edit.as_mut().unwrap().draft = crop;
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
    let brackets=h.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::LineSegment { stroke,.. } if stroke.width==2.5 && stroke.color==egui::Color32::WHITE)).count();
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
    let draft = h.editor.crop_edit.as_ref().unwrap().draft;
    h.editor.crop_edit.as_mut().unwrap().guides = false;
    let no_guides = h.frame(vec![]);
    assert_eq!(no_guides.composition, resized.composition);
    assert_eq!(h.editor.crop_edit.as_ref().unwrap().draft, draft);
    assert!(!h.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::LineSegment { stroke,.. } if stroke.width==0.5 && stroke.color==egui::Color32::WHITE)));
}

struct Harness {
    editor: Editor,
    context: egui::Context,
    cursor: egui::CursorIcon,
    shapes: Vec<egui::epaint::ClippedShape>,
    _receiver: mpsc::Receiver<Command>,
}
impl Harness {
    fn new(
        turns: crate::geometry::QuarterTurns,
        roll: RollOrientation,
        ratio: Option<f64>,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (_send, messages) = mpsc::channel();
        let mut editor = Editor::state(
            commands,
            messages,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        );
        editor.roll_orientation = roll;
        editor.photo_orientation = PhotoOrientation {
            turns,
            flips: Flips {
                horizontal: true,
                vertical: false,
            },
        };
        editor.crop_edit = Some(CropEdit {
            draft: Crop::from_region(PATCH),
            viewport: Viewport::default(),
            ratio,
            drag: None,
            layout: None,
            guides: true,
        });
        Self {
            editor,
            context: egui::Context::default(),
            cursor: egui::CursorIcon::Default,
            shapes: Vec::new(),
            _receiver: receiver,
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
                        let (canvas, response) = ui.allocate_exact_size(
                            ui.available_size(),
                            egui::Sense::click_and_drag(),
                        );
                        let composition = Composition::with_crop(
                            AREA,
                            self.editor.roll_orientation,
                            self.editor.photo_orientation,
                            Some(self.editor.crop_edit.as_ref().unwrap().draft),
                        )
                        .unwrap();
                        let mut layout =
                            self.editor
                                .viewport
                                .layout_crop(canvas.shrink(20.), composition, 1.);
                        layout.canvas = canvas;
                        let layout = self.editor.crop_interaction(ui, &response, layout);
                        self.editor.paint_crop(ui, layout);
                        result = Some(layout);
                    });
            },
        );
        self.cursor = output.platform_output.cursor_icon;
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
                assert_eq!(h.editor.crop_edit.as_ref().unwrap().draft, wanted);
                assert!(h.editor.crop.is_none());
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
                    assert!(h.editor.crop.is_none());
                    h.press(end, false);
                    assert!(h.editor.crop_edit.as_ref().unwrap().drag.is_none());
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
