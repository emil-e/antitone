use super::*;

#[test]
fn creation_has_a_compact_hint_and_viewer_identity_with_cancel_and_no_apply_step() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for (size, width, name) in [
            (egui::vec2(1100., 800.), 315., "normal"),
            (egui::vec2(800., 600.), 270., "narrow"),
        ] {
            set_layout(&mut h, size, width);
            for radial in [false, true] {
                let before = h.read(|editor| editor.document.state.project.clone());
                let history = h.read(|editor| {
                    editor
                        .history_label(crate::editor::history::Direction::Undo)
                        .map(str::to_owned)
                });
                arm(&mut h, radial);
                let title = if radial {
                    "New radial gradient"
                } else {
                    "New linear gradient"
                };
                h.read(|editor| {
                    assert!(editor.creating_mask());
                    assert!(editor.mask_preview_geometry().is_none());
                    assert!(editor.current_tool().unwrap().is::<mask_editor::MaskTool>());
                });
                let canvas = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.canvas);
                let hint = h
                    .get_by_label(if radial {
                        "Drag to draw a radial gradient."
                    } else {
                        "Drag to draw a linear gradient."
                    })
                    .rect();
                assert!(
                    hint.center().x < canvas.left() || hint.center().x > canvas.right(),
                    "a compact creation hint stays visible beside the canvas"
                );
                assert!(
                    h.query_by_label(title).is_none(),
                    "no duplicate inspector tool heading"
                );
                let viewer = h.get_by_label(&format!("Active tool: {title}")).rect();
                assert!(
                    viewer.center().x >= canvas.left() && viewer.center().x <= canvas.right(),
                    "viewer names the active mode"
                );
                let cancel = h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel");
                assert!(!cancel.accesskit_node().is_disabled());
                assert!(
                    h.query_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                        .is_none()
                );
                h.event(egui::Event::PointerGone);
                h.step();
                save_capture(
                    &h.render().unwrap(),
                    &format!(
                        "mask-creation-{}-{name}",
                        if radial { "radial" } else { "linear" }
                    ),
                );
                click(&mut h, "Cancel");
                h.read(|editor| {
                    assert!(!editor.masking());
                    assert_eq!(editor.document.state.project, before);
                    assert_eq!(
                        editor.history_label(crate::editor::history::Direction::Undo),
                        history.as_deref()
                    );
                });
            }
        }
    });
}

#[test]
fn creation_allows_roll_grading_and_cancel_retains_it() {
    fn controls(h: &mut Harness<'_, Editor>, disabled: bool) {
        for label in ["Gamma", "White balance wheel", "Exposure"] {
            let control = if label == "White balance wheel" {
                h.get_by_label(label)
            } else {
                h.get_by_role_and_label(egui::accesskit::Role::SpinButton, label)
            };
            control.scroll_to_me();
            h.try_run_realtime().unwrap();
            let control = if label == "White balance wheel" {
                h.get_by_label(label)
            } else {
                h.get_by_role_and_label(egui::accesskit::Role::SpinButton, label)
            };
            assert_eq!(control.accesskit_node().is_disabled(), disabled, "{label}");
        }
    }
    workflow::calibrated_workflow(|mut h, _, _directory| {
        stage(&mut h, "Roll");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
            .click();
        h.step();
        h.try_run_realtime().unwrap();
        for (radial, exposure) in [(false, "0.25"), (true, "0.5")] {
            controls(&mut h, false);
            arm(&mut h, radial);
            let before = h.read(|editor| editor.document.state.project.clone());
            stage(&mut h, "Roll");
            controls(&mut h, false);
            h.read(|editor| {
                assert!(editor.creating_mask());
                assert_eq!(editor.document.state.project, before);
            });
            workflow::enter_numeric(&mut h, "Exposure", exposure);
            let graded = h.read(|editor| {
                assert!(editor.creating_mask());
                editor.document.project.clone()
            });
            click(&mut h, "Cancel");
            controls(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, graded));
            h.read(|editor| {
                assert_eq!(
                    editor.document.project.roll.exposure,
                    exposure.parse::<f64>().unwrap()
                )
            });
        }
    });
}

#[test]
fn creation_captures_primary_drawing_without_disabling_independent_navigation() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for radial in [false, true] {
            arm(&mut h, radial);
            click(&mut h, "100%");
            click(&mut h, "Zoom in");
            click(&mut h, "Zoom in");
            let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
            let point = image.center();
            let before_pan = h.read(|editor| editor.document.visit.canvas.viewport);
            h.hover_at(point);
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Middle,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.hover_at(point + egui::vec2(25., 20.));
            h.step();
            h.event(egui::Event::PointerButton {
                pos: point + egui::vec2(25., 20.),
                button: egui::PointerButton::Middle,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.read(|editor| {
                assert_ne!(
                    editor.document.visit.canvas.viewport, before_pan,
                    "armed creation retains explicit panning"
                );
                assert!(editor.creating_mask() && !editor.mask_dragging());
            });
            let layout = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            let start = layout.canvas.center() - egui::vec2(30., 25.);
            let end = start + egui::vec2(50., 40.);
            let anchor = layout
                .composition
                .frame_to_source(layout.frame_point(start));
            let viewport = h.read(|editor| editor.document.visit.canvas.viewport);
            hold_drag(&mut h, start, end);
            h.read(|editor| {
                assert!(editor.mask_dragging());
                assert_eq!(
                    editor.document.visit.canvas.viewport, viewport,
                    "drawing does not also pan"
                );
            });
            h.key_press(egui::Key::Z);
            h.step();
            h.event(egui::Event::Zoom(1.25));
            h.step();
            h.read(|editor| {
                assert_ne!(editor.document.visit.canvas.viewport, viewport);
                assert_ne!(
                    editor.active_canvas().last_layout.unwrap().0.image,
                    layout.image
                );
            });
            pointer(&mut h, end, false);
            h.read(|editor| {
                assert!(editor.masking() && !editor.creating_mask() && !editor.mask_dragging());
                let layout = editor.active_canvas().last_layout.unwrap().0;
                match accepted_mask(editor) {
                    AdjustmentLayerCoverage::Linear { start, end: fixed } => {
                        assert_eq!(fixed, anchor);
                        assert_eq!(
                            start,
                            layout.composition.frame_to_source(layout.frame_point(end)),
                            "release uses the navigated view, not the press-time mapping"
                        );
                    }
                    AdjustmentLayerCoverage::Radial { center, radii, .. } => {
                        assert_eq!(center, anchor);
                        let center = layout.composition.source_to_frame(center);
                        let pointer = layout.frame_point(end);
                        let radius = (pointer.0[0] - center.0[0]).hypot(pointer.0[1] - center.0[1]);
                        assert!((radii[0] - radius).abs() < 1e-9);
                        assert_eq!(radii[0], radii[1]);
                    }
                    AdjustmentLayerCoverage::Global {} => unreachable!(),
                }
            });
            let (before, layout, grab) = h.read(|editor| {
                let before = accepted_mask(editor);
                let layout = editor.active_canvas().last_layout.unwrap().0;
                let point = match before {
                    AdjustmentLayerCoverage::Linear { end, .. } => end,
                    AdjustmentLayerCoverage::Radial { center, .. } => center,
                    AdjustmentLayerCoverage::Global {} => unreachable!(),
                };
                (
                    before,
                    layout,
                    layout.screen_frame_point(layout.composition.source_to_frame(point)),
                )
            });
            pointer(&mut h, grab, true);
            h.key_press(egui::Key::Z);
            h.step();
            h.event(egui::Event::Zoom(1.8));
            h.step();
            let end = layout.canvas.center() + egui::vec2(45., 30.);
            pointer(&mut h, end, false);
            h.read(|editor| {
                assert!(!editor.mask_dragging());
                let current = editor.active_canvas().last_layout.unwrap().0;
                let press = layout.composition.frame_to_source(layout.frame_point(grab));
                let release = current
                    .composition
                    .frame_to_source(current.frame_point(end));
                let delta = glam::DVec2::from_array(release.0) - glam::DVec2::from_array(press.0);
                let mut expected = before;
                match &mut expected {
                    AdjustmentLayerCoverage::Linear { end, .. }
                    | AdjustmentLayerCoverage::Radial { center: end, .. } => {
                        *end = crate::geometry::SourcePoint(
                            (glam::DVec2::from_array(end.0) + delta).to_array(),
                        );
                    }
                    AdjustmentLayerCoverage::Global {} => unreachable!(),
                }
                assert_ne!(expected, before);
                assert_eq!(
                    accepted_mask(editor),
                    expected,
                    "refinement uses the current view mapping too"
                );
            });
            click(&mut h, "Fit");
            h.read(|editor| assert!(editor.document.visit.canvas.viewport.zoom.is_none()));
        }
    });
}

#[test]
fn creation_release_is_one_undoable_layer_and_replacement_does_not_publish_a_held_gesture() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for radial in [false, true] {
            arm(&mut h, radial);
            let before = h.read(|editor| editor.document.state.project.clone());
            let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
            hold_drag(
                &mut h,
                image.center() - egui::vec2(30., 25.),
                image.center() + egui::vec2(30., 25.),
            );
            h.read(|editor| {
                assert!(editor.mask_dragging());
                assert_eq!(editor.document.state.project, before);
            });
            save_capture(
                &h.render().unwrap(),
                if radial {
                    "mask-creation-radial-drawing"
                } else {
                    "mask-creation-linear-drawing"
                },
            );
            pointer(&mut h, image.center() + egui::vec2(30., 25.), false);
            h.try_run_realtime().unwrap();
            let accepted = h.read(|editor| {
                assert!(editor.masking() && !editor.creating_mask() && !editor.mask_dragging());
                if !radial {
                    let mask = accepted_mask(editor);
                    let layout = editor.active_canvas().last_layout.unwrap().0;
                    let at_press = layout
                        .composition
                        .frame_to_source(layout.frame_point(image.center() - egui::vec2(30., 25.)));
                    let at_release = layout
                        .composition
                        .frame_to_source(layout.frame_point(image.center() + egui::vec2(30., 25.)));
                    assert_eq!(mask.coverage(at_press).unwrap(), 1.);
                    assert_eq!(mask.coverage(at_release).unwrap(), 0.);
                }
                let source = editor.document.visit.current_source.unwrap();
                let previous = before
                    .source(source)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap();
                let layers = editor
                    .document
                    .state
                    .project
                    .source(source)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap();
                assert_eq!(layers.layers.len(), previous.layers.len() + 1);
                assert_eq!(layers.next_id, previous.next_id + 1);
                let created = layers
                    .layers
                    .iter()
                    .filter(|layer| !previous.layers.iter().any(|old| old.id == layer.id))
                    .collect::<Vec<_>>();
                assert_eq!(created.len(), 1);
                assert_eq!(
                    editor.adjustment_layer_target(),
                    super::super::super::adjustment_layers::Target::Layer(created[0].id)
                );
                editor.document.state.project.clone()
            });
            h.read(|editor| {
                assert!(
                    editor
                        .application_menu_state()
                        .enabled(crate::platform::menu::Action::Undo)
                )
            });
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.step();
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| assert_eq!(editor.document.state.project, before));
            h.key_press_modifiers(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            );
            h.step();
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| assert_eq!(editor.document.state.project, accepted));

            arm(&mut h, radial);
            let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
            hold_drag(
                &mut h,
                image.center() - egui::vec2(35., 20.),
                image.center() + egui::vec2(35., 20.),
            );
            let retired = h.read(|editor| Rc::downgrade(&editor.mask_creation().unwrap()));
            h.update(Editor::begin_crop);
            h.step();
            pointer(&mut h, image.center() + egui::vec2(35., 20.), false);
            h.read(|editor| {
                assert!(
                    retired.upgrade().is_none(),
                    "replacement drops the creation owner"
                );
                assert!(
                    editor.cropping(),
                    "late physical release must not end the successor"
                );
                assert!(!editor.masking());
                assert_eq!(editor.document.state.project, accepted);
            });
            click(&mut h, "Cancel");
            h.read(|editor| assert!(!editor.cropping()));
        }
    });
}
