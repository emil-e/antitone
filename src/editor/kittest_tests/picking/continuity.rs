use super::*;

fn setup(test: impl FnOnce(Harness<'_, Editor>)) {
    editor_harness(|mut h, _| {
        h.update(|editor| editor.document.project.input = InputSettings::Digital);
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_opened_photo(&mut h);
        stage(&mut h, "Photo");
        workflow::wait_settled(&mut h);
        test(h);
    });
}

#[test]
fn layer_name_history_preserves_pending_working_view_while_picking() {
    metadata_replay(false);
}

#[test]
fn layer_name_history_preserves_completed_working_view_while_picking() {
    metadata_replay(true);
}

fn metadata_replay(completed: bool) {
    setup(|mut h| {
        let before = h.read(|editor| editor.document.project.clone());
        h.update(|editor| {
            editor
                .edit_adjustment_layers_named("Layer name", |layers| {
                    layers.layers[0].name = "Renamed".into();
                    Ok(())
                })
                .unwrap();
        });
        let after = h.read(|editor| editor.document.project.clone());
        assert_ne!(before, after);
        open(&mut h, PickerCase::Wb);
        let hold = (!completed).then(crate::background::testing::pause);
        if !completed {
            h.update(|editor| {
                let (layout, density) = editor.active_canvas().last_layout.unwrap();
                editor.document.visit.canvas.viewport.zoom_at(
                    layout,
                    density,
                    layout.scale * density * 2.,
                    layout.canvas.center(),
                );
            });
            h.step();
            wait_for(&mut h, |editor| editor.document.visit.working_view.updating);
        } else {
            workflow::wait_settled(&mut h);
        }
        let (revision, image) = h.read(|editor| {
            (
                editor.document.visit.working_view.revision(),
                editor.document.visit.working_view.displayed.clone(),
            )
        });
        for (direction, expected) in [(Direction::Undo, &before), (Direction::Redo, &after)] {
            h.enter(|editor, cx| editor.restore_history(direction, cx));
            h.step();
            h.read(|editor| {
                assert_eq!(&editor.document.project, expected);
                assert_eq!(editor.document.visit.working_view.revision(), revision);
                if completed {
                    assert!(Arc::ptr_eq(
                        editor
                            .document
                            .visit
                            .working_view
                            .displayed
                            .as_ref()
                            .unwrap(),
                        image.as_ref().unwrap()
                    ));
                } else {
                    assert!(editor.document.visit.working_view.updating);
                }
                assert_eq!(editor.test_pick(), Some(PickerCase::Wb.display()));
            });
        }
        drop(hold);
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                &editor.canvas_frame.displayed.as_ref().unwrap().view,
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap(),
            ))
        });
    });
}

#[test]
fn look_history_updates_the_normal_positive_view_without_ending_the_picker() {
    setup(|mut h| {
        let asset = crate::editor::kittest_tests::looks::constant_asset(0.4);
        h.enter(|editor, cx| editor.select_look(look::Source::Loaded(asset.clone()), cx));
        wait_for(&mut h, |editor| editor.document.lut.pending.is_none());
        open(&mut h, PickerCase::Wb);
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        let selected =
            h.read(|editor| editor.canvas_frame.displayed.as_ref().unwrap().view.clone());
        h.update(Editor::clear_look);
        for expected in [None, Some(asset.lut().id()), None] {
            workflow::wait_settled(&mut h);
            h.render().unwrap();
            h.read(|editor| {
                assert_eq!(
                    editor
                        .canvas_frame
                        .displayed
                        .as_ref()
                        .unwrap()
                        .view
                        .capture
                        .look_id(),
                    expected
                );
                assert_eq!(editor.test_pick(), Some(PickerCase::Wb.display()));
                if expected.is_none() {
                    assert!(!Arc::ptr_eq(
                        &selected,
                        &editor.canvas_frame.displayed.as_ref().unwrap().view
                    ));
                }
            });
            if expected.is_none() {
                h.enter(|editor, cx| editor.restore_history(Direction::Undo, cx));
            } else {
                h.enter(|editor, cx| editor.restore_history(Direction::Redo, cx));
            }
        }
    });
}

#[test]
fn layer_grading_replaces_the_working_image_while_picking() {
    changed_inputs(|editor| {
        editor
            .edit_adjustment_layers(|layers| {
                layers.layers[0].adjustment.photo.exposure = 0.75;
                Ok(())
            })
            .unwrap();
    });
}

#[test]
fn cropping_replaces_the_working_image_while_picking() {
    changed_inputs(|editor| {
        let area = editor.selected_area().unwrap();
        editor
            .accept_crop(Crop::from_region(Rect {
                x: area.x + 8,
                y: area.y + 8,
                width: area.width - 16,
                height: area.height - 16,
            }))
            .unwrap();
    });
}

#[test]
fn output_target_replaces_the_working_image_while_picking() {
    changed_inputs(|editor| {
        let mut edit = editor.edit_project("Output target").unwrap();
        edit.update(|project| project.output.tiff = crate::project::TiffTarget::Srgb);
        editor.commit_project(edit).unwrap();
    });
}

fn changed_inputs(edit: impl FnOnce(&mut Editor)) {
    setup(|mut h| {
        open(&mut h, PickerCase::Wb);
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        let (revision, image) = h.read(|editor| {
            (
                editor.document.visit.working_view.revision(),
                editor.canvas_frame.displayed.as_ref().unwrap().view.clone(),
            )
        });
        let hold = crate::background::testing::pause();
        h.update(edit);
        h.step();
        h.read(|editor| assert_ne!(editor.document.visit.working_view.revision(), revision));
        drop(hold);
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        h.read(|editor| {
            let displayed = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert!(!Arc::ptr_eq(displayed, &image));
            assert!(
                displayed.capture
                    == *editor
                        .document
                        .visit
                        .working_view
                        .completed_capture()
                        .unwrap()
            );
            assert_eq!(editor.test_pick(), Some(PickerCase::Wb.display()));
            assert!(editor.document.visit.working_view.error.is_none());
        });
    });
}

#[test]
fn photographic_history_replaces_the_working_image_without_ending_the_picker() {
    setup(|mut h| {
        h.update(|editor| {
            let mut edit = editor.edit_project("Roll exposure").unwrap();
            edit.update(|project| project.roll.exposure = 0.75);
            editor.commit_project(edit).unwrap();
        });
        open(&mut h, PickerCase::Wb);
        workflow::wait_settled(&mut h);
        let previous = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .displayed
                .clone()
                .unwrap()
        });
        h.enter(|editor, cx| editor.restore_history(Direction::Undo, cx));
        workflow::wait_settled(&mut h);
        h.render().unwrap();
        h.read(|editor| {
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert_eq!(editor.test_pick(), Some(PickerCase::Wb.display()));
            assert!(
                editor.canvas_frame.displayed.as_ref().unwrap().view.capture != previous.capture
            );
        });
    });
}

#[test]
fn reinterpretation_retires_picker_and_its_captured_input_scope() {
    setup(|mut h| {
        open(&mut h, PickerCase::Wb);
        h.update(|editor| {
            editor.reinterpret_photo();
        });
        h.read(|editor| assert!(!editor.picking()));
    });
}
