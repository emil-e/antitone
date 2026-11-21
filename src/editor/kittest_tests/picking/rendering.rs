use super::*;

#[test]
fn completing_initial_calibration_keeps_source_until_the_user_selects_positive() {
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_opened_photo(&mut h);
        h.read(|editor| {
            assert!(editor.calibration().is_none());
            assert!(editor.document.source);
        });
        for (kind, point, expected) in [
            (PickerCase::Dmin, egui::pos2(32., 32.), 3000.),
            (PickerCase::Dmax, egui::pos2(96., 32.), 128.),
        ] {
            open(&mut h, kind);
            let layout = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            drag(
                &mut h,
                layout.screen_point(point - egui::vec2(8., 8.)),
                layout.screen_point(point + egui::vec2(8., 8.)),
            );
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                .click();
            h.step();
            wait_for(&mut h, |editor| {
                let measured = if kind == PickerCase::Dmin {
                    editor.dmin()
                } else {
                    editor.dmax()
                };
                measured.is_some() && !editor.measurement_pending() && !editor.interaction_blocked()
            });
            workflow::wait_settled(&mut h);
            h.render().unwrap();
            h.read(|editor| {
                let measured = if kind == PickerCase::Dmin {
                    editor.dmin()
                } else {
                    editor.dmax()
                }
                .unwrap();
                assert!(
                    (measured - DVec3::splat(expected / 4031.))
                        .abs()
                        .max_element()
                        <= 2. / 65535.
                );
                assert!(editor.test_pick().is_none());
                assert!(editor.document.source);
                assert!(editor.canvas_inspection());
                assert!(
                    editor
                        .canvas_frame
                        .displayed
                        .as_ref()
                        .unwrap()
                        .view
                        .capture
                        .is_inspection()
                );
            });
        }
        h.read(|editor| assert!(editor.calibration().is_some()));
        h.get_by_label("View source").click();
        h.step();
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        h.read(|editor| {
            assert!(!editor.document.source);
            assert!(
                !editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .unwrap()
                    .view
                    .capture
                    .is_inspection()
            );
        });
    });
}

#[test]
fn working_rendering_progresses_while_modal_input_is_reserved() {
    editor_harness(|mut h, _| {
        h.update(|editor| editor.document.project.input = InputSettings::Digital);
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_opened_photo(&mut h);
        open(&mut h, PickerCase::Wb);
        workflow::wait_settled(&mut h);
        let before = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .displayed
                .clone()
                .unwrap()
        });
        h.update(|editor| {
            editor.photo_settings_mut().exposure = 0.75;
            editor.set_test_busy(true);
        });
        h.step();
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        h.read(|editor| {
            assert!(editor.interaction_blocked());
            let displayed = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert!(displayed.capture != before.capture);
            assert!(Arc::ptr_eq(
                displayed,
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap()
            ));
        });
        h.update(|editor| editor.set_test_busy(false));
    });
}

#[test]
fn picker_preserves_display_and_manual_source_navigation() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        let asset = crate::editor::kittest_tests::looks::constant_asset(0.4);
        h.enter(|editor, cx| editor.select_look(look::Source::Loaded(asset.clone()), cx));
        wait_for(&mut h, |editor| editor.document.lut.pending.is_none());
        h.update(|editor| {
            let area = editor.selected_area().unwrap();
            editor
                .accept_crop(Crop {
                    center: [
                        f64::from(area.x) + f64::from(area.width) * 0.5,
                        f64::from(area.y) + f64::from(area.height) * 0.5,
                    ],
                    size: [area.width / 2, area.height / 2],
                    angle: 0.11,
                })
                .unwrap();
            editor.photo_orientation_mut().turn_clockwise();
        });
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut h, size, width);
            for source in [false, true] {
                for kind in [
                    PickerCase::Dmin,
                    PickerCase::Dmax,
                    PickerCase::Gamma,
                    PickerCase::GammaPivot,
                    PickerCase::RollWb,
                    PickerCase::Wb,
                ] {
                    stage(
                        &mut h,
                        if matches!(kind, PickerCase::Wb) {
                            "Photo"
                        } else {
                            "Roll"
                        },
                    );
                    h.update(|editor| {
                        editor.document.source = source;
                        editor.document.visit.canvas.viewport = Default::default();
                    });
                    h.step();
                    h.update(|editor| {
                        let (layout, density) = editor.active_canvas().last_layout.unwrap();
                        let viewport = &mut editor.document.visit.canvas.viewport;
                        viewport.zoom_at(
                            layout,
                            density,
                            layout.scale * density * 1.5,
                            layout.canvas.center(),
                        );
                        let layout =
                            viewport.layout_composition(layout.canvas, layout.composition, density);
                        viewport.pan(layout, density, egui::vec2(15., -10.));
                    });
                    h.step();
                    workflow::wait_settled(&mut h);
                    h.render().unwrap();
                    let (view, composition, viewport, project) = h.read(|editor| {
                        (
                            editor.canvas_frame.displayed.as_ref().unwrap().view.clone(),
                            editor.active_canvas().last_layout.unwrap().0.composition,
                            editor.document.visit.canvas.viewport,
                            editor.document.project.clone(),
                        )
                    });
                    open(&mut h, kind);
                    h.render().unwrap();
                    h.read(|editor| {
                        assert_eq!(editor.test_pick(), Some(kind.display()));
                        assert_eq!(editor.document.source, source);
                        assert_eq!(editor.canvas_inspection(), source);
                        assert_eq!(editor.document.visit.canvas.viewport, viewport);
                        assert_eq!(
                            editor.active_canvas().last_layout.unwrap().0.composition,
                            composition
                        );
                        assert!(
                            Arc::ptr_eq(
                                &editor.canvas_frame.displayed.as_ref().unwrap().view,
                                &view
                            ),
                            "opening {kind:?} must retain the rendered image"
                        );
                        assert_eq!(editor.document.project, project);
                    });
                    h.get_by_label("View source").click();
                    h.step();
                    workflow::wait_settled(&mut h);
                    h.render().unwrap();
                    h.read(|editor| {
                        assert_eq!(editor.test_pick(), Some(kind.display()));
                        assert_eq!(editor.document.source, !source);
                        assert_eq!(editor.canvas_inspection(), !source);
                        assert!(
                            editor.canvas_frame.displayed.as_ref().unwrap().view.capture
                                != view.capture
                        );
                        assert_eq!(editor.document.visit.canvas.viewport, viewport);
                        assert_eq!(
                            editor.active_canvas().last_layout.unwrap().0.composition,
                            composition
                        );
                    });
                    h.get_by_label("100%").click();
                    h.step();
                    let navigated = h.read(|editor| {
                        assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(1.));
                        assert_eq!(editor.test_pick(), Some(kind.display()));
                        editor.document.visit.canvas.viewport
                    });
                    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                        .click();
                    h.step();
                    h.read(|editor| {
                        assert!(editor.test_pick().is_none());
                        assert_eq!(editor.document.visit.canvas.viewport, navigated);
                        assert_eq!(editor.document.source, !source);
                        assert_eq!(editor.document.project, project);
                    });
                }
            }
        }
    });
}
