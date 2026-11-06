use super::*;

#[test]
fn option_and_shift_resize_all_handles_through_orientation_and_straightening() {
    for turns in [
        crate::geometry::QuarterTurns::Zero,
        crate::geometry::QuarterTurns::One,
        crate::geometry::QuarterTurns::Two,
        crate::geometry::QuarterTurns::Three,
    ] {
        for angle in [0., 0.23] {
            for alt in [false, true] {
                for shift in [false, true] {
                    for index in 0..8 {
                        let mut h = Harness::new(turns, RollOrientation::default(), None);
                        h.editor.document.crop_edit_mut().unwrap().draft =
                            Crop::from_region(PATCH).straightened(AREA, angle).unwrap();
                        h.modifiers = egui::Modifiers {
                            alt,
                            shift,
                            ..Default::default()
                        };
                        let before = h.frame(vec![]);
                        let (start, handle) = handles(before.image)[index];
                        h.press(start, true);
                        let end = start
                            - egui::vec2(f32::from(handle[0]) * 35., f32::from(handle[1]) * 18.);
                        let after = h.frame(vec![egui::Event::PointerMoved(end)]);
                        let crop = after.composition.crop();
                        assert_ne!(
                            crop.size,
                            before.composition.crop().size,
                            "resize must not be a no-op"
                        );
                        crop.coverage().unwrap().bounds(AREA).unwrap();
                        assert_eq!(crop.angle, before.composition.crop().angle);
                        if alt {
                            for axis in 0..2 {
                                assert!(
                                    (crop.center[axis] - before.composition.crop().center[axis])
                                        .abs()
                                        <= 0.51,
                                    "center moved: {turns:?}, {angle}, {handle:?}: {crop:?}"
                                );
                            }
                        }
                        if shift {
                            let initial = before.composition.size().map(f64::from);
                            let size = after.composition.size().map(f64::from);
                            assert!(
                                (size[0] - size[1] * initial[0] / initial[1]).abs() <= 1.5,
                                "ratio changed: {initial:?} -> {size:?}"
                            );
                        } else if handle[0] == 0 {
                            assert_eq!(after.composition.size()[0], before.composition.size()[0]);
                        } else if handle[1] == 0 {
                            assert_eq!(after.composition.size()[1], before.composition.size()[1]);
                        }
                        assert!(h.editor.document.crop_edit().unwrap().ratio.is_none());
                    }
                }
            }
        }
    }
}

#[test]
fn every_free_handle_tracks_pointer_motion_and_preserves_the_grab_offset() {
    for density in [1., 2.] {
        for turns in [
            crate::geometry::QuarterTurns::Zero,
            crate::geometry::QuarterTurns::One,
        ] {
            for angle in [0., 0.23] {
                for alt in [false, true] {
                    for index in 0..8 {
                        let mut h = Harness::new(turns, Default::default(), None);
                        h.density = density;
                        h.editor.document.crop_edit_mut().unwrap().draft =
                            Crop::from_region(PATCH).straightened(AREA, angle).unwrap();
                        h.modifiers.alt = alt;
                        let before = h.frame(vec![]);
                        let (handle_start, handle) = handles(before.image)[index];
                        let grab_offset = egui::vec2(2., -2.);
                        let start = handle_start + grab_offset;
                        h.press(start, true);
                        for distance in [-20., -35., -12., 8., 0.] {
                            let delta = egui::vec2(
                                f32::from(handle[0]) * distance,
                                f32::from(handle[1]) * distance,
                            );
                            let point = start + delta;
                            let after = h.frame(vec![egui::Event::PointerMoved(point)]);
                            let actual = handles(after.image)[index].0;
                            let error = actual + grab_offset - point;
                            assert!(
                                error.x.abs() < before.scale * 0.26 + 0.001
                                    && error.y.abs() < before.scale * 0.26 + 0.001,
                                "handle detached: {density}, {turns:?}, {angle}, {alt}, {handle:?}, {distance}: {actual:?} vs {point:?}"
                            );
                            assert_eq!(after.scale, before.scale, "resizing must not zoom");
                            assert!(after.image.center().distance(before.image.center()) < 0.001);
                            let held = h.frame(vec![]);
                            assert_eq!(held.composition.crop(), after.composition.crop());
                            assert!(handles(held.image)[index].0.distance(actual) < 0.001);
                        }
                        h.press(start, false);
                        let released = h.frame(vec![]);
                        assert_eq!(released.composition.crop(), before.composition.crop());
                        assert!(released.image.min.distance(before.image.min) < 0.001);
                    }
                }
            }
        }
    }
}

#[test]
fn modifiers_can_change_during_resize_without_changing_the_aspect_setting() {
    let mut h = Harness::new(Default::default(), Default::default(), None);
    let before = h.frame(vec![]);
    let start = before.image.max;
    let end = start - egui::vec2(80., 12.);
    h.press(start, true);
    let free = h.frame(vec![egui::Event::PointerMoved(end)]);
    let initial = before.composition.size().map(f64::from);
    let ratio_error = |layout: CanvasLayout| {
        let size = layout.composition.size().map(f64::from);
        (size[0] - size[1] * initial[0] / initial[1]).abs()
    };
    assert!(
        ratio_error(free) > 3.,
        "control must visibly change the aspect"
    );
    h.modifiers.shift = true;
    let locked = h.frame(vec![]);
    assert!(ratio_error(locked) <= 1.5);
    h.modifiers.alt = true;
    let centered = h.frame(vec![]);
    for axis in 0..2 {
        assert!(
            (centered.composition.crop().center[axis] - before.composition.crop().center[axis])
                .abs()
                <= 0.51
        );
    }
    h.modifiers = egui::Modifiers::NONE;
    let restored = h.frame(vec![]);
    assert_eq!(restored.composition.crop(), free.composition.crop());
    assert!(h.editor.document.crop_edit().unwrap().ratio.is_none());
    h.press(end, false);
    let next = h.frame(vec![]);
    h.press(next.image.max, true);
    h.modifiers.shift = true;
    let current = h.frame(vec![egui::Event::PointerMoved(
        next.image.max - egui::vec2(20., 5.),
    )]);
    let size = current.composition.size().map(f64::from);
    let previous = next.composition.size().map(f64::from);
    assert!(
        (size[0] - size[1] * previous[0] / previous[1]).abs() <= 1.5,
        "a new gesture must capture the new current aspect"
    );
}

#[test]
fn centered_expansion_stops_at_source_without_translating_the_crop() {
    for angle in [0., 0.31] {
        for (index, shift) in (0..8).flat_map(|index| [false, true].map(|shift| (index, shift))) {
            let mut h = Harness::new(Default::default(), Default::default(), None);
            h.editor.document.crop_edit_mut().unwrap().draft =
                Crop::from_region(PATCH).straightened(AREA, angle).unwrap();
            h.modifiers = egui::Modifiers {
                alt: true,
                shift,
                ..Default::default()
            };
            let before = h.frame(vec![]);
            let (start, handle) = handles(before.image)[index];
            h.press(start, true);
            let after = h.frame(vec![egui::Event::PointerMoved(
                start + egui::vec2(f32::from(handle[0]) * 1000., f32::from(handle[1]) * 800.),
            )]);
            let crop = after.composition.crop();
            assert_ne!(crop.size, before.composition.crop().size);
            for (axis, direction) in handle.into_iter().enumerate() {
                if !shift && direction == 0 {
                    assert_eq!(
                        after.composition.size()[axis],
                        before.composition.size()[axis],
                        "source-boundary clipping must not resize an ungrabbed axis"
                    );
                }
            }
            let coverage = crop.coverage().unwrap();
            coverage.bounds(AREA).unwrap();
            let distance_to_edge = coverage
                .corners()
                .iter()
                .flat_map(|point| {
                    [
                        point.0[0] - f64::from(AREA.x),
                        f64::from(AREA.x + AREA.width) - point.0[0],
                        point.0[1] - f64::from(AREA.y),
                        f64::from(AREA.y + AREA.height) - point.0[1],
                    ]
                })
                .fold(f64::INFINITY, f64::min);
            assert!(
                distance_to_edge < 1.5,
                "expansion stopped before reaching source coverage: {crop:?}"
            );
            for axis in 0..2 {
                assert!((crop.center[axis] - before.composition.crop().center[axis]).abs() <= 0.51);
            }
        }
    }
}

#[test]
fn ordinary_expansion_preserves_the_opposite_source_anchor_at_image_bounds() {
    for angle in [0., 0.31] {
        for index in 0..8 {
            let mut h = Harness::new(crate::geometry::QuarterTurns::One, Default::default(), None);
            h.editor.document.crop_edit_mut().unwrap().draft =
                Crop::from_region(PATCH).straightened(AREA, angle).unwrap();
            let before = h.frame(vec![]);
            let (start, handle) = handles(before.image)[index];
            let opposite = |layout: CanvasLayout| {
                handles(layout.image)
                    .into_iter()
                    .find(|(_, other)| *other == handle.map(|axis| -axis))
                    .unwrap()
                    .0
            };
            let anchor = before.source_point(opposite(before));
            h.press(start, true);
            let after = h.frame(vec![egui::Event::PointerMoved(
                start + egui::vec2(f32::from(handle[0]) * 1000., f32::from(handle[1]) * 800.),
            )]);
            assert_ne!(
                after.composition.crop().size,
                before.composition.crop().size
            );
            after
                .composition
                .crop()
                .coverage()
                .unwrap()
                .bounds(AREA)
                .unwrap();
            assert!(
                after.source_point(opposite(after)).distance(anchor) <= 1.01,
                "boundary moved the opposite source anchor: {angle}, {handle:?}"
            );
        }
    }
}

#[test]
fn changing_aspect_keeps_magnification_and_screen_center() {
    let composition = Composition::with_crop(
        AREA,
        Default::default(),
        Default::default(),
        Some(Crop::from_region(PATCH)),
    )
    .unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 400.));
    let fit = crop_canvas(canvas);
    let mut viewport = Viewport::default();
    let mut original = viewport.layout_crop(fit, composition, 1.);
    original.canvas = canvas;
    let magnification = 1.25;
    original.scale *= magnification;
    original.image = egui::Rect::from_center_size(
        canvas.center() + egui::vec2(23., -17.),
        original.image.size() * magnification,
    );
    assert!((original.image.height() - fit.height() * magnification).abs() < 0.001);
    let next = composition
        .cropped(Crop {
            size: [60, 12],
            ..composition.crop()
        })
        .unwrap();
    let after = resized_layout(original, next);
    assert_eq!(after.scale, original.scale);
    assert!((after.image.width() - original.image.width()).abs() < 0.001);
    assert!((after.image.height() - original.image.height() * 0.3).abs() < 0.001);
    assert!(after.image.center().distance(original.image.center()) < 0.001);
}

#[test]
fn resize_retains_screen_pan_and_physical_magnification() {
    let mut h = Harness::new(Default::default(), Default::default(), None);
    let fitted = h.frame(vec![]);
    let viewport = h.editor.document.crop_viewport_mut().unwrap();
    viewport.zoom_at(fitted, 1., fitted.scale * 1.25, fitted.canvas.center());
    let zoomed = h.frame(vec![]);
    h.editor
        .document
        .crop_viewport_mut()
        .unwrap()
        .pan(zoomed, 1., egui::vec2(-45., 27.));
    let before = h.frame(vec![]);
    let offset = before.image.center() - before.canvas.center();
    assert!(offset.length() > 20., "fixture must actually pan");
    h.modifiers.shift = true;
    let start = before.image.center_top();
    assert!(before.canvas.contains(start));
    let end = start + egui::vec2(0., 30.);
    h.press(start, true);
    let after = h.frame(vec![egui::Event::PointerMoved(end)]);
    assert_ne!(after.composition.size(), before.composition.size());
    assert!((after.image.center() - after.canvas.center() - offset).length() < 0.001);
    assert_eq!(after.scale, before.scale);
    assert!(after.image.width() < before.image.width());
    assert!(after.image.height() < before.image.height());
    assert!(after.image.center_top().distance(end) < before.scale * 0.26);
    h.press(end, false);
    let released = h.frame(vec![]);
    assert!(released.image.min.distance(after.image.min) < 0.001);
    assert!(released.image.max.distance(after.image.max) < 0.001);
}
