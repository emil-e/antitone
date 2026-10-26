use super::*;

fn layout(turns: usize, flipped: bool) -> CanvasLayout {
    let area = Rect {
        x: 120,
        y: 73,
        width: 2300,
        height: 1900,
    };
    let mut roll = RollOrientation::default();
    for _ in 0..turns {
        roll.turn_clockwise();
    }
    let mut photo = PhotoOrientation::default();
    photo.flips.horizontal = flipped;
    let crop = Crop::from_region(area).straightened(area, 0.13).unwrap();
    let composition = Composition::with_crop(area, roll, photo, Some(crop)).unwrap();
    Viewport::default().layout_composition(
        egui::Rect::from_min_size(egui::pos2(20., 30.), egui::vec2(800., 600.)),
        composition,
        1.,
    )
}

fn mask() -> AdjustmentLayerCoverage {
    AdjustmentLayerCoverage::Radial {
        center: SourcePoint([1120.125, 900.375]),
        radii: [480.25, 310.5],
        angle: 0.41,
        feather: 0.8,
    }
}

fn handle(mask: AdjustmentLayerCoverage, layout: CanvasLayout, target: Handle) -> egui::Pos2 {
    handles(mask, layout)
        .into_iter()
        .find(|(_, handle)| *handle == target)
        .unwrap()
        .0
}

fn close(a: SourcePoint, b: SourcePoint) {
    assert!(
        (DVec2::from_array(a.0) - DVec2::from_array(b.0)).length() < 0.001,
        "{a:?} != {b:?}"
    );
}

#[test]
fn radial_creation_keeps_the_pressed_center_and_expands_a_circle_through_every_orientation() {
    for turns in 0..4 {
        for flipped in [false, true] {
            let layout = layout(turns, flipped);
            let start = egui::pos2(layout.image.left() - 35., layout.image.top() + 40.);
            let end = layout.image.right_bottom() + egui::vec2(10., 20.);
            let draft = RadialDrag::begin(None, layout, start, controls::DEFAULT_RADIAL_FEATHER)
                .update(end);
            draft.validate().unwrap();
            let AdjustmentLayerCoverage::Radial {
                center,
                radii,
                angle,
                feather,
            } = draft
            else {
                unreachable!()
            };
            assert_eq!(feather, 0.8);
            let screen = screen_point(layout, center);
            assert!(screen.distance(start) < 0.001);
            let a = screen_point(layout, position(center, radii, angle, DVec2::X));
            let b = screen_point(layout, position(center, radii, angle, DVec2::Y));
            assert!((a.y - screen.y).abs() < 0.001);
            assert!((b.x - screen.x).abs() < 0.001);
            assert_eq!(radii[0], radii[1]);
            assert!((a.distance(screen) - end.distance(start)).abs() < 0.001);
            assert!((b.distance(screen) - end.distance(start)).abs() < 0.001);
            assert!(
                (normalized_radius(source_point(layout, end), center, radii, angle) - 1.).abs()
                    < 1e-6
            );
            assert_eq!(draft.coverage(center).unwrap(), 1.);
        }
    }
}

#[test]
fn radial_handles_capture_translation_axes_rotation_and_feather_without_press_jumps() {
    for flipped in [false, true] {
        let layout = layout(1, flipped);
        let original = mask();
        let AdjustmentLayerCoverage::Radial {
            center,
            radii,
            angle,
            feather,
        } = original
        else {
            unreachable!()
        };
        for target in [
            Handle::Center,
            Handle::Axis(0, 1.),
            Handle::Axis(0, -1.),
            Handle::Axis(1, 1.),
            Handle::Axis(1, -1.),
            Handle::Rotate,
            Handle::Feather,
        ] {
            let start = handle(original, layout, target);
            let drag = RadialDrag::begin(Some(original), layout, start, feather);
            assert_eq!(drag.handle, target);
            assert_eq!(
                drag.update(start),
                original,
                "pressing a handle must preserve precise saved values"
            );
            let press = source_point(layout, start);
            let delta = match target {
                Handle::Axis(axis, sign) => axes(angle)[axis] * sign * 37.25,
                _ => DVec2::new(35., -21.),
            };
            let end = screen_point(
                layout,
                SourcePoint((DVec2::from_array(press.0) + delta).to_array()),
            );
            if matches!(target, Handle::Center | Handle::Axis(..)) {
                let AdjustmentLayerCoverage::Radial {
                    center: moved,
                    radii: resized,
                    angle: rotated,
                    feather: softness,
                } = drag.update(end)
                else {
                    unreachable!()
                };
                assert_eq!(rotated, angle);
                assert_eq!(softness, feather);
                match target {
                    Handle::Center => {
                        close(
                            moved,
                            SourcePoint((DVec2::from_array(center.0) + delta).to_array()),
                        );
                        assert_eq!(resized, radii);
                    }
                    Handle::Axis(axis, _) => {
                        assert_eq!(moved, center);
                        assert!((resized[axis] - radii[axis] - 37.25).abs() < 0.001);
                        assert_eq!(resized[1 - axis], radii[1 - axis]);
                    }
                    _ => unreachable!(),
                }
            }
        }
        let start = handle(original, layout, Handle::Rotate);
        let press = DVec2::from_array(source_point(layout, start).0) - DVec2::from_array(center.0);
        let delta = glam::DMat2::from_angle(0.7) * press;
        let end = screen_point(
            layout,
            SourcePoint((DVec2::from_array(center.0) + delta).to_array()),
        );
        let AdjustmentLayerCoverage::Radial {
            center: moved,
            radii: resized,
            angle: rotated,
            feather: softness,
        } = RadialDrag::begin(Some(original), layout, start, feather).update(end)
        else {
            unreachable!()
        };
        assert_eq!(moved, center);
        assert_eq!(resized, radii);
        assert_eq!(softness, feather);
        assert!((rotated - angle - 0.7).abs() < 1e-6);
        let start = handle(original, layout, Handle::Feather);
        for (radius, expected) in [(1.1, 0.), (0.6, 0.4), (0.2, 0.8), (0., 1.)] {
            let end = screen_point(
                layout,
                position(
                    center,
                    radii,
                    angle,
                    DVec2::splat(radius * std::f64::consts::FRAC_1_SQRT_2),
                ),
            );
            let AdjustmentLayerCoverage::Radial {
                center: moved,
                radii: resized,
                angle: rotated,
                feather,
            } = RadialDrag::begin(Some(original), layout, start, 0.8).update(end)
            else {
                unreachable!()
            };
            assert_eq!(moved, center);
            assert_eq!(resized, radii);
            assert_eq!(rotated, angle);
            assert!((feather - expected).abs() < 1e-6);
        }
    }
}

#[test]
fn feather_handle_tracks_the_inner_boundary_and_full_feather_keeps_center_movement_available() {
    let layout = layout(0, false);
    let AdjustmentLayerCoverage::Radial {
        center,
        radii,
        angle,
        ..
    } = mask()
    else {
        unreachable!()
    };
    for feather in [0., 0.25, 0.8, 0.99] {
        let mask = AdjustmentLayerCoverage::Radial {
            center,
            radii,
            angle,
            feather,
        };
        let point = handle(mask, layout, Handle::Feather);
        assert!(
            (normalized_radius(source_point(layout, point), center, radii, angle) - (1. - feather))
                .abs()
                < 1e-6
        );
    }
    let full = AdjustmentLayerCoverage::Radial {
        center,
        radii,
        angle,
        feather: 1.,
    };
    assert!(
        !handles(full, layout)
            .iter()
            .any(|(_, handle)| *handle == Handle::Feather)
    );
    assert_eq!(
        handle_at(Some(full), layout, screen_point(layout, center)),
        Handle::Center
    );
}

#[test]
fn zero_size_drafts_are_invalid_but_positive_small_ovals_and_hard_edges_keep_existing_semantics() {
    let layout = layout(0, false);
    let start = layout.image.center();
    let zero = RadialDrag::begin(None, layout, start, 0.8).update(start);
    assert!(zero.validate().is_err());
    assert_eq!(handle_at(Some(zero), layout, start), Handle::New);
    let horizontal =
        RadialDrag::begin(None, layout, start, 0.8).update(start + egui::vec2(20., 0.));
    horizontal.validate().unwrap();
    let tiny = AdjustmentLayerCoverage::Radial {
        center: SourcePoint([120.5, 80.5]),
        radii: [1e-6, 2e-6],
        angle: 0.,
        feather: 0.,
    };
    tiny.validate().unwrap();
    assert_eq!(tiny.coverage(SourcePoint([120.5, 80.5])).unwrap(), 1.);
    assert_eq!(tiny.coverage(SourcePoint([120.500002, 80.5])).unwrap(), 0.);
}
