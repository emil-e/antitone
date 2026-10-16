use super::*;
use crate::geometry::SourcePoint;
use glam::DVec2;

fn seed(editor: &mut Editor) {
    adjustments::install_locals(editor);
    let area = editor.ready.as_ref().unwrap().area;
    let source = editor
        .document
        .project
        .source_mut(editor.current_source.unwrap())
        .unwrap();
    source.local_adjustments.as_mut().unwrap().adjustments[0]
        .adjustment
        .mask = GradientMask::Radial {
        center: SourcePoint([
            area.x as f64 + area.width as f64 * 0.5,
            area.y as f64 + area.height as f64 * 0.5,
        ]),
        radii: [area.width as f64 * 0.25, area.height as f64 * 0.18],
        angle: 0.3,
        feather: 0.8,
    };
}

fn screen(editor: &Editor, point: SourcePoint) -> egui::Pos2 {
    let layout = editor.last_layout.unwrap().0;
    layout.screen_frame_point(layout.composition.source_to_frame(point))
}

fn point(mask: GradientMask, unit: DVec2) -> SourcePoint {
    let GradientMask::Radial {
        center,
        radii,
        angle,
        ..
    } = mask
    else {
        unreachable!()
    };
    let (sin, cos) = angle.sin_cos();
    SourcePoint(
        (DVec2::from_array(center.0)
            + DVec2::new(cos, sin) * radii[0] * unit.x
            + DVec2::new(-sin, cos) * radii[1] * unit.y)
            .to_array(),
    )
}

fn open_mask_controls(harness: &mut Harness<'_, Editor>) {
    harness.try_run_realtime().unwrap();
    adjustments::open_drawer(harness);
}

#[test]
fn radial_release_accepts_defaults_and_hard_edges_while_escape_discards_tiny_gestures() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    for (size, width) in [
        (egui::vec2(1100., 800.), 315.),
        (egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        let before = harness.state().document.project.clone();
        arm(&mut harness, true);
        let center = harness.state().last_layout.unwrap().0.image.center();
        hold_drag(&mut harness, center, center + egui::vec2(0.01, 0.02));
        assert!(
            harness.state().mask_edit.as_ref().unwrap().valid(),
            "no arbitrary small-radius floor"
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        pointer(&mut harness, center + egui::vec2(0.01, 0.02), false);
        assert_eq!(harness.state().document.project, before);
        arm(&mut harness, true);
        let layout = harness.state().last_layout.unwrap().0;
        let pressed = layout.image.center();
        let expected_center = layout
            .composition
            .frame_to_source(layout.frame_point(pressed));
        drag(&mut harness, pressed, pressed + egui::vec2(70., 45.));
        assert!(
            harness.state().mask_edit.is_none(),
            "{}",
            harness.state().status
        );
        let record = adjustments::locals(harness.state())
            .adjustments
            .last()
            .unwrap();
        let GradientMask::Radial {
            center,
            radii,
            feather,
            ..
        } = record.adjustment.mask
        else {
            unreachable!()
        };
        assert_eq!(center, expected_center);
        assert_eq!(radii[0], radii[1]);
        assert_eq!(feather, 0.8);
        assert_eq!(record.adjustment.strength, 1.);
        assert_eq!(record.adjustment.photo, PhotoSettings::default());
        open_mask_controls(&mut harness);
        workflow::enter_numeric(&mut harness, "Feather", "0");
        let GradientMask::Radial { feather, .. } = adjustments::locals(harness.state())
            .adjustments
            .last()
            .unwrap()
            .adjustment
            .mask
        else {
            unreachable!()
        };
        assert_eq!(feather, 0.);
        assert!(harness.state().mask_edit.is_none());
        harness
            .state()
            .validate_document_candidate(&harness.state().document)
            .unwrap();
    }
}

#[test]
fn selected_radial_handles_commit_axes_rotation_and_feather_without_changing_other_controls() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    seed(harness.state_mut());
    {
        let editor = harness.state_mut();
        let area = editor.ready.as_ref().unwrap().area;
        editor.roll_orientation.turn_clockwise();
        editor.photo_orientation.flips.horizontal = true;
        editor.crop = Some(Crop::from_region(area).straightened(area, 0.13).unwrap());
        editor.sync_document();
    }
    harness.step();
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        click(&mut harness, "Sky");
        workflow::wait_settled(&mut harness);
        let before = harness.state().document.project.clone();
        let original = accepted_mask(harness.state());
        let center_screen = screen(harness.state(), point(original, DVec2::ZERO));
        assert!(
            harness.state().mask_handles_available(),
            "handles unavailable"
        );
        assert!(
            harness
                .state()
                .last_layout
                .unwrap()
                .0
                .canvas
                .contains(center_screen)
        );
        assert!(
            harness
                .state()
                .mask_pointer_interest(harness.state().last_layout.unwrap().0, center_screen),
            "target={:?} mask={original:?}",
            harness.state().adjustment_target()
        );
        hold_drag(
            &mut harness,
            center_screen,
            center_screen + egui::vec2(20., -15.),
        );
        assert_eq!(harness.state().document.project, before);
        assert!(
            harness.state().mask_preview_source().is_some(),
            "gesture={} error={:?} status={}",
            harness.state().mask_edit.is_some(),
            harness.state().mask_edit.as_ref().and_then(MaskEdit::error),
            harness.state().status
        );
        save_capture(
            &harness.render().unwrap(),
            &format!("local-radial-direct-{name}-offscreen"),
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        pointer(&mut harness, center_screen + egui::vec2(20., -15.), false);
        assert_eq!(harness.state().document.project, before);
        let axis = screen(harness.state(), point(original, DVec2::X));
        let along = (axis - center_screen).normalized();
        drag(&mut harness, axis, axis + along * 12.);
        assert!(harness.state().mask_edit.is_none());
        let resized = accepted_mask(harness.state());
        let GradientMask::Radial {
            center: original_center,
            radii: original_radii,
            angle: original_angle,
            feather: original_feather,
        } = original
        else {
            unreachable!()
        };
        let GradientMask::Radial {
            center,
            radii,
            angle,
            feather,
        } = resized
        else {
            unreachable!()
        };
        assert_eq!(center, original_center);
        assert_eq!(angle, original_angle);
        assert_eq!(feather, original_feather);
        assert!(radii[0] > original_radii[0]);
        assert_eq!(radii[1], original_radii[1]);
        let center_screen = screen(harness.state(), center);
        let axis = screen(harness.state(), point(resized, DVec2::X));
        let handle = axis + (axis - center_screen).normalized() * 28.;
        drag(
            &mut harness,
            handle,
            center_screen + egui::emath::Rot2::from_angle(0.4) * (handle - center_screen),
        );
        let rotated = accepted_mask(harness.state());
        let GradientMask::Radial {
            angle: moved_angle,
            center: moved_center,
            radii: moved_radii,
            ..
        } = rotated
        else {
            unreachable!()
        };
        assert!((moved_angle - angle).abs() > 0.2);
        assert_eq!(moved_center, center);
        assert_eq!(moved_radii, radii);
        let diamond = screen(
            harness.state(),
            point(
                rotated,
                DVec2::splat((1. - feather) * std::f64::consts::FRAC_1_SQRT_2),
            ),
        );
        let hard = screen(
            harness.state(),
            point(rotated, DVec2::splat(1.1 * std::f64::consts::FRAC_1_SQRT_2)),
        );
        drag(&mut harness, diamond, hard);
        let GradientMask::Radial {
            feather: hard_edge, ..
        } = accepted_mask(harness.state())
        else {
            unreachable!()
        };
        assert_eq!(hard_edge, 0.);
        assert_eq!(
            harness
                .query_all_by_role_and_label(egui::accesskit::Role::SpinButton, "Rotation")
                .count(),
            0
        );
        open_mask_controls(&mut harness);
        workflow::enter_numeric(&mut harness, "Feather", "0.375");
        assert!(
            harness.state().mask_edit.is_none(),
            "numeric Enter is not a geometry-session action"
        );
        let GradientMask::Radial { feather, angle, .. } = accepted_mask(harness.state()) else {
            unreachable!()
        };
        assert_eq!(feather, 0.375);
        assert_eq!(angle, moved_angle);
        let mut expected = before;
        expected
            .source_mut(harness.state().current_source.unwrap())
            .unwrap()
            .local_adjustments
            .as_mut()
            .unwrap()
            .adjustments[0]
            .adjustment
            .mask = accepted_mask(harness.state());
        assert_eq!(
            harness.state().document.project,
            expected,
            "IDs, allocator, photographic values, other locals and composition survive geometry edits"
        );
    }
}
