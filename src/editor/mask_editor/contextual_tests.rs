use super::*;

mod history_tests;

fn start(editor: &mut Editor) -> AdjustmentLayerId {
    editor.tab = 1;
    let source = editor.document.visit.current_source.unwrap();
    let id = editor
        .document
        .project
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0]
        .id;
    editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
    editor.begin_linear_mask(Some(id));
    editor.test_mask_preview(AdjustmentLayerCoverage::Linear {
        start: SourcePoint([100., 70.]),
        end: SourcePoint([149., 99.]),
    });
    assert!(editor.mask_preview_source().is_some());
    assert!(editor.tool_interacting());
    assert!(editor.current_tool().unwrap().is::<MaskTool>());
    id
}

#[test]
fn gradient_refinement_does_not_require_an_executable_image() {
    pollster::block_on(async {
        for bad_calibration in [true, false] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            let source = editor.document.visit.current_source.unwrap();
            if bad_calibration {
                if let InputSettings::Negative { dmin, dmax, .. } =
                    &mut editor.document.project.input
                {
                    *dmax = *dmin;
                }
            } else {
                let layer = &mut editor
                    .document
                    .project
                    .source_mut(source)
                    .unwrap()
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .layers[1]
                    .adjustment;
                if let AdjustmentLayerCoverage::Radial { radii, .. } = &mut layer.mask {
                    radii[0] = f64::MIN_POSITIVE;
                } else {
                    panic!("fixture has a radial layer");
                }
            }
            assert!(
                editor
                    .positive_processing(
                        editor.input(),
                        editor.document.project.roll,
                        editor.photo_settings()
                    )
                    .is_err()
            );
            let before = editor.document.state.project.clone();
            let id = start(&mut editor);
            let mask = editor.mask_preview_geometry().unwrap();
            editor.finish_mask(true);
            let mut expected = before.clone();
            expected
                .source_mut(source)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap()
                .layers
                .iter_mut()
                .find(|layer| layer.id == id)
                .unwrap()
                .adjustment
                .mask = mask;
            assert_eq!(editor.document.state.project, expected);
            assert!(
                editor
                    .positive_processing(
                        editor.input(),
                        editor.document.project.roll,
                        editor.photo_settings()
                    )
                    .is_err()
            );
            editor.restore_history(history::Direction::Undo, &UiContext::render_only());
            assert_eq!(editor.document.state.project, before);
        }
    });
}

#[test]
fn visit_retirement_and_layer_selection_discard_only_the_unfinished_refinement() {
    pollster::block_on(async {
        for depart in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;

            let before = editor.document.state.project.clone();
            let history = editor.history_label(history::Direction::Undo);
            start(&mut editor);
            if depart {
                editor.replace_photo_scope(editor.document.visit.current_source);
            } else {
                editor.select_adjustment_layer(adjustment_layers::Target::None);
            }
            assert!(!editor.masking());
            assert!(editor.mask_preview_source().is_none());
            editor.finish_mask(true);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.history_label(history::Direction::Undo), history);
        }
    });
}

#[test]
fn refinement_changes_only_geometry_and_preserves_later_accepted_controls() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = start(&mut editor);
        editor
            .edit_adjustment_layers(|layers| {
                layers
                    .layers
                    .iter_mut()
                    .find(|layer| layer.id == id)
                    .unwrap()
                    .adjustment
                    .photo
                    .exposure = 0.75;
                Ok(())
            })
            .unwrap();
        let graded = editor.document.state.project.clone();
        let source = editor.document.visit.current_source.unwrap();
        assert_eq!(
            graded
                .source(source)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers
                .iter()
                .find(|layer| layer.id == id)
                .unwrap()
                .adjustment
                .photo
                .exposure,
            0.75
        );
        let mut expected = graded.clone();
        expected
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .unwrap()
            .adjustment
            .mask = editor.mask_preview_geometry().unwrap();
        assert!(editor.mask_preview_source().is_some());
        editor.finish_mask(true);
        assert!(editor.masking());
        assert!(!editor.mask_dragging());
        assert_eq!(editor.document.state.project, expected);
        editor.restore_history(history::Direction::Undo, &UiContext::render_only());
        assert_eq!(editor.document.state.project, graded);
    });
}

#[test]
fn document_admission_retires_the_gesture_before_preparation_is_polled() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        start(&mut editor);
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, _| editor.new_project());
            app.read(|editor| {
                assert!(
                    !editor.masking(),
                    "replacement retires the unfinished refinement"
                );
                assert_ne!(editor.document.state.project, before);
                assert!(editor.document.project.sources.is_empty());
            });
        });
    });
}

#[test]
fn removed_or_changed_mask_targets_cannot_be_overwritten_by_a_held_refinement() {
    pollster::block_on(async {
        for remove in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            let id = start(&mut editor);
            editor
                .edit_adjustment_layers(|layers| {
                    if remove {
                        layers.remove(id)?;
                    } else {
                        layers
                            .layers
                            .iter_mut()
                            .find(|layer| layer.id == id)
                            .unwrap()
                            .adjustment
                            .mask = AdjustmentLayerCoverage::Linear {
                            start: SourcePoint([2., 3.]),
                            end: SourcePoint([18., 22.]),
                        };
                    }
                    Ok(())
                })
                .unwrap();
            let accepted = editor.document.state.project.clone();
            let history = editor
                .history_label(history::Direction::Undo)
                .map(str::to_owned);
            assert!(editor.mask_preview_source().is_none());
            editor.finish_mask(true);
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                history.as_deref()
            );
            assert!(
                editor.status.starts_with("Mask rejected:"),
                "{}",
                editor.status
            );
        }
    });
}

#[test]
fn cancelled_new_keeps_refinement_and_actual_new_retires_it_without_applying() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        start(&mut editor);
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| editor.file_action(FileAction::New, cx));
            app.run_until(Duration::from_secs(5), |editor| editor.dialogs.is_open());
            app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
            app.run_until(Duration::from_secs(5), |editor| {
                !editor.document.files.pending()
            });
            app.read(|editor| {
                assert!(editor.masking());
                assert_eq!(editor.document.state.project, before);
            });
            app.enter(|editor, cx| editor.file_action(FileAction::New, cx));
            app.run_until(Duration::from_secs(5), |editor| editor.dialogs.is_open());
            app.update(|editor| editor.dialogs.answer_for_test("Discard changes"));
            app.run_until(Duration::from_secs(5), |editor| {
                !editor.document.files.pending()
            });
            app.update(|editor| editor.finish_mask(true));
            app.read(|editor| {
                assert!(!editor.masking());
                assert!(editor.document.project.sources.is_empty());
                assert_eq!(editor.history_label(history::Direction::Undo), None);
            });
        });
    });
}

#[test]
fn snapshots_and_roll_commands_remain_available_during_private_refinement() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        assert!(
            editor.document_command_available(),
            "idle mask editing does not block file actions"
        );
        start(&mut editor);
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.read(|editor| {
                assert!(editor.tool_interacting());
                assert!(editor.document_command_available());
                for action in [
                    crate::platform::menu::Action::New,
                    crate::platform::menu::Action::AddScans,
                    crate::platform::menu::Action::Open,
                    crate::platform::menu::Action::Close,
                    crate::platform::menu::Action::Quit,
                ] {
                    assert!(editor.application_menu_state().enabled(action));
                }
            });
            app.update(Editor::open_export_dialog);
            app.read(|editor| {
                assert!(!editor.document.files.pending());
                assert!(editor.document.export.form.is_some());
                assert!(editor.masking());
                assert_eq!(editor.document.state.project, before);
            });
            app.update(|editor| {
                editor.document.export.form = None;
                editor.finish_mask(false);
            });
            app.read(|editor| assert!(editor.document_command_available()));
            app.update(Editor::open_export_dialog);
            app.read(|editor| {
                assert!(
                    editor.document.export.form.is_some(),
                    "export remains reachable immediately after the individual gesture"
                );
                assert_eq!(editor.document.state.project, before);
            });
        });
    });
}
