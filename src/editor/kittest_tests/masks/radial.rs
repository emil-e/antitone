use super::*;
use crate::geometry::SourcePoint;
use glam::DVec2;

fn seed(editor: &mut Editor) {
    adjustment_layers::install_layers(editor);
    let area = editor.document.visit.ready.as_ref().unwrap().area;
    let source = editor
        .document
        .state
        .project
        .source_mut(editor.document.visit.current_source.unwrap())
        .unwrap();
    source.adjustment_layers.as_mut().unwrap().layers[0]
        .adjustment
        .mask = AdjustmentLayerCoverage::Radial {
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
    let layout = editor.document.visit.last_layout.unwrap().0;
    layout.screen_frame_point(layout.composition.source_to_frame(point))
}

fn point(mask: AdjustmentLayerCoverage, unit: DVec2) -> SourcePoint {
    let AdjustmentLayerCoverage::Radial {
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
    adjustment_layers::show_layers(harness);
}

#[test]
fn radial_release_accepts_defaults_and_hard_edges_while_escape_discards_tiny_gestures() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            let before = harness.read(|editor| editor.document.state.project.clone());
            arm(&mut harness, true);
            let center =
                harness.read(|editor| editor.document.visit.last_layout.unwrap().0.image.center());
            hold_drag(&mut harness, center, center + egui::vec2(0.01, 0.02));
            harness.read(|editor| {
                assert!(
                    editor.document.visit.mask_edit.as_ref().unwrap().valid(),
                    "no arbitrary small-radius floor"
                )
            });
            harness.key_press(egui::Key::Escape);
            harness.step();
            pointer(&mut harness, center + egui::vec2(0.01, 0.02), false);
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            arm(&mut harness, true);
            let layout = harness.read(|editor| editor.document.visit.last_layout.unwrap().0);
            let pressed = layout.image.center();
            let expected_center = layout
                .composition
                .frame_to_source(layout.frame_point(pressed));
            drag(&mut harness, pressed, pressed + egui::vec2(70., 45.));
            harness.read(|editor| {
                assert!(
                    editor.document.visit.mask_edit.is_none(),
                    "{}",
                    editor.status
                )
            });
            let layers = harness.read(adjustment_layers::layers);
            let record = layers.layers.last().unwrap();
            let AdjustmentLayerCoverage::Radial {
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
            assert_eq!(record.adjustment.photo, AdjustmentLayerSettings::default());
            open_mask_controls(&mut harness);
            workflow::enter_numeric(&mut harness, "Feather", "0");
            let AdjustmentLayerCoverage::Radial { feather, .. } = harness.read(|editor| {
                adjustment_layers::layers(editor)
                    .layers
                    .last()
                    .unwrap()
                    .adjustment
                    .mask
            }) else {
                unreachable!()
            };
            assert_eq!(feather, 0.);
            harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
            harness.read(|editor| {
                editor
                    .validate_document_candidate(&editor.document)
                    .unwrap()
            });
        }
    });
}

#[test]
fn selected_radial_handles_commit_axes_rotation_and_feather_without_changing_other_controls() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(seed);
        harness.update(|editor| {
            let area = editor.document.visit.ready.as_ref().unwrap().area;
            editor.document.roll_orientation.turn_clockwise();
            editor.document.visit.photo_orientation.flips.horizontal = true;
            editor.document.visit.crop =
                Some(Crop::from_region(area).straightened(area, 0.13).unwrap());
            editor.sync_document();
        });
        harness.step();
        for (name, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            click(&mut harness, "Sky");
            workflow::wait_settled(&mut harness);
            let before = harness.read(|editor| editor.document.state.project.clone());
            let original = harness.read(accepted_mask);
            let center_screen = harness.read(|editor| screen(editor, point(original, DVec2::ZERO)));
            harness.read(|editor| assert!(editor.mask_handles_available(), "handles unavailable"));
            harness.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .last_layout
                        .unwrap()
                        .0
                        .canvas
                        .contains(center_screen)
                )
            });
            harness.read(|editor| {
                assert!(
                    editor.mask_pointer_interest(
                        editor.document.visit.last_layout.unwrap().0,
                        center_screen
                    ),
                    "target={:?} mask={original:?}",
                    editor.adjustment_layer_target()
                )
            });
            hold_drag(
                &mut harness,
                center_screen,
                center_screen + egui::vec2(20., -15.),
            );
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            harness.read(|editor| {
                assert!(
                    editor.mask_preview_source().is_some(),
                    "gesture={} error={:?} status={}",
                    editor.document.visit.mask_edit.is_some(),
                    editor
                        .document
                        .visit
                        .mask_edit
                        .as_ref()
                        .and_then(MaskEdit::error),
                    editor.status
                )
            });
            save_capture(
                &harness.render().unwrap(),
                &format!("local-radial-direct-{name}-offscreen"),
            );
            harness.key_press(egui::Key::Escape);
            harness.step();
            pointer(&mut harness, center_screen + egui::vec2(20., -15.), false);
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            let axis = harness.read(|editor| screen(editor, point(original, DVec2::X)));
            let along = (axis - center_screen).normalized();
            drag(&mut harness, axis, axis + along * 12.);
            harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
            let resized = harness.read(accepted_mask);
            let AdjustmentLayerCoverage::Radial {
                center: original_center,
                radii: original_radii,
                angle: original_angle,
                feather: original_feather,
            } = original
            else {
                unreachable!()
            };
            let AdjustmentLayerCoverage::Radial {
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
            let center_screen = harness.read(|editor| screen(editor, center));
            let axis = harness.read(|editor| screen(editor, point(resized, DVec2::X)));
            let handle = axis + (axis - center_screen).normalized() * 28.;
            drag(
                &mut harness,
                handle,
                center_screen + egui::emath::Rot2::from_angle(0.4) * (handle - center_screen),
            );
            let rotated = harness.read(accepted_mask);
            let AdjustmentLayerCoverage::Radial {
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
            let diamond = harness.read(|editor| {
                screen(
                    editor,
                    point(
                        rotated,
                        DVec2::splat((1. - feather) * std::f64::consts::FRAC_1_SQRT_2),
                    ),
                )
            });
            let hard = harness.read(|editor| {
                screen(
                    editor,
                    point(rotated, DVec2::splat(1.1 * std::f64::consts::FRAC_1_SQRT_2)),
                )
            });
            drag(&mut harness, diamond, hard);
            let AdjustmentLayerCoverage::Radial {
                feather: hard_edge, ..
            } = harness.read(accepted_mask)
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
            harness.read(|editor| {
                assert!(
                    editor.document.visit.mask_edit.is_none(),
                    "numeric Enter is not a geometry-session action"
                )
            });
            let AdjustmentLayerCoverage::Radial { feather, angle, .. } =
                harness.read(accepted_mask)
            else {
                unreachable!()
            };
            assert_eq!(feather, 0.375);
            assert_eq!(angle, moved_angle);
            let mut expected = before;
            harness.read(|editor| {
                expected
                    .source_mut(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .mask = accepted_mask(editor)
            });
            harness.read(|editor| assert_eq!(
            editor.document.state.project,
            expected,
            "IDs, allocator, photographic values, other layers and composition survive geometry edits"
        ));
        }
    });
}
