use super::*;
mod failure_tests;
use crate::editor::{export, files};
use crate::platform::menu::Action;

fn authored_editor() -> Editor {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.set_dmin(Some(DVec3::splat(0.8)));
    editor.set_dmax(Some(DVec3::splat(0.02)));
    editor.document.source = false;
    editor.tab = 1;
    let source = editor.document.visit.current_source.unwrap();
    let path = editor.document.source_path(source).unwrap();
    editor.document.path = Some(path.with_extension("antitone"));
    editor.document.project.source_mut(source).unwrap().path = path.file_name().unwrap().into();
    editor.document.capture_source_files().unwrap();
    editor
        .dust_repairs_mut(source)
        .insert(
            SupportMask::new([60, 110], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [30, 0],
        )
        .unwrap();
    editor
}

#[test]
fn save_as_during_a_real_search_writes_only_capture_and_preserves_publication_on_either_side_of_acceptance()
 {
    for complete_before_write in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pending.antitone");
        with_search(authored_editor(), |app, gate| {
            let stroke = app.enter(|editor, cx| {
                editor.begin_dust_cleanup();
                queue(editor, [100, 110], cx)
            });
            let call = gate.next(app);
            let locator = app.read(|editor| {
                editor
                    .document
                    .project
                    .source(stroke.source)
                    .unwrap()
                    .path
                    .clone()
            });
            let hold = crate::background::testing::pause();
            let captured = app.enter(|editor, cx| {
                for action in [Action::Save, Action::SaveAs, Action::Export] {
                    assert!(editor.application_menu_state().enabled(action));
                }
                assert!(!editor.file_available());
                let captured = editor
                    .document
                    .state
                    .project
                    .source(stroke.source)
                    .unwrap()
                    .repairs
                    .clone();
                editor.save_destination_action(
                    files::Destination {
                        path: path.clone(),
                        policy: crate::project::Save::New,
                        confirm_replace: false,
                    },
                    None,
                    cx,
                );
                assert!(editor.document.write.pending());
                captured
            });
            app.pump();
            assert!(!call.cancelled.is_cancelled());
            if complete_before_write {
                call.complete(Ok(SearchOutcome::Candidate([20, 0])));
                settled(app);
                assert!(app.read(|editor| editor.document.write.pending()));
                drop(hold);
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.write.pending()
                });
            } else {
                drop(hold);
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.write.pending()
                });
                assert!(
                    app.read(|editor| editor.dust_search_pending() && !editor.document.dirty())
                );
                call.complete(Ok(SearchOutcome::Candidate([20, 0])));
                settled(app);
            }
            assert_eq!(
                crate::project::Project::load(&path)
                    .unwrap()
                    .source(stroke.source)
                    .unwrap()
                    .repairs,
                captured
            );
            app.read(|editor| {
                assert!(editor.dusting());
                assert!(!editor.dialogs.is_open(), "{}", editor.status);
                assert_eq!(editor.document.path.as_ref(), Some(&path));
                assert_ne!(
                    editor.document.project.source(stroke.source).unwrap().path,
                    locator
                );
                let repairs = editor.dust_repairs(stroke.source).unwrap().repairs();
                assert_eq!(repairs.len(), 2);
                assert_eq!(repairs[1].support, stroke.support);
                assert_eq!(repairs[1].offset, [20, 0]);
                assert!(editor.document.dirty());
                assert_eq!(
                    editor.history_label(history::Direction::Undo),
                    Some("Dust mask")
                );
            });
        });
    }
}

fn open_form(app: &mut TestApplication<'_, Editor>, folder: &std::path::Path) {
    app.update(|editor| {
        editor.open_export_dialog();
        let form = editor
            .document
            .export
            .form
            .as_mut()
            .expect("export form with pending Dust");
        form.draft.folder = folder.to_owned();
        form.draft.kind = export::Kind::Exr;
        form.draft.precision = ExrPrecision::Full;
    });
}

fn confirm_form(app: &mut TestApplication<'_, Editor>) {
    let context = egui::Context::default();
    app.enter(|editor, cx| {
        frame(editor, &context, vec![], cx);
        let labels = frame(editor, &context, vec![], cx);
        let button = labels
            .iter()
            .rev()
            .find(|(label, _)| label == "Export 1 photo")
            .unwrap()
            .1
            .center();
        frame(
            editor,
            &context,
            crate::editor::ui::test_support::pointer(button, true),
            cx,
        );
        frame(
            editor,
            &context,
            crate::editor::ui::test_support::pointer(button, false),
            cx,
        );
        assert!(editor.document.export.form.is_none());
        assert!(
            editor.document.export.operation.is_some(),
            "{}",
            editor.status
        );
    });
}

#[test]
fn export_confirmation_captures_search_results_only_if_already_accepted_and_freezes_later_grades() {
    for complete_before_confirmation in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        with_search(authored_editor(), |app, gate| {
            app.enter(|editor, cx| {
                editor.begin_dust_cleanup();
                queue(editor, [100, 110], cx);
            });
            let call = gate.next(app);
            open_form(app, directory.path());
            let call = if complete_before_confirmation {
                call.complete(Ok(SearchOutcome::Candidate([20, 0])));
                settled(app);
                None
            } else {
                Some(call)
            };
            let request = app.update(|editor| {
                let form = editor.document.export.form.take().unwrap();
                let request = editor.capture_export(&form.draft);
                editor.document.export.form = Some(form);
                request.unwrap()
            });
            let recipe = request.photos[0].recipe.clone();
            let path = request.photos[0].path.clone();
            assert_eq!(
                recipe.repairs.len(),
                if complete_before_confirmation { 2 } else { 1 }
            );
            let expected = app.read(|editor| {
                let scan = crate::scan::Scan::open(
                    &editor.document.source_path(recipe.neutral.source).unwrap(),
                    editor.document.project.acquisition_wb(),
                )
                .unwrap();
                let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
                    editor
                        .document
                        .project
                        .calibration()
                        .unwrap()
                        .unwrap()
                        .validate()
                        .unwrap(),
                    recipe.roll,
                    recipe.grade.clone(),
                    scan.camera_to_ap1,
                )
                .unwrap();
                crate::color::ap1_to_ap0()
                    * pipeline
                        .apply(scan.pixel(32, 32).unwrap(), [32, 32])
                        .unwrap()
                        .rgb
            });
            let hold = crate::background::testing::pause();
            confirm_form(app);
            if let Some(call) = call {
                // The form/worker reservation owns input, not this search's right to publish.
                call.complete(Ok(SearchOutcome::Candidate([20, 0])));
                settled(app);
            }
            app.update(|editor| {
                let mut edit = editor.edit_photo("After export capture").unwrap();
                let source = editor.document.visit.current_source.unwrap();
                edit.update(|project| {
                    project
                        .source_mut(source)
                        .unwrap()
                        .adjustment_layers
                        .as_mut()
                        .unwrap()
                        .layers[0]
                        .adjustment
                        .photo
                        .exposure += 1.
                });
                editor.commit_project(edit).unwrap();
                assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 2);
            });
            drop(hold);
            app.run_until(Duration::from_secs(20), |editor| {
                editor.document.export.operation.is_none()
            });
            app.read(|editor| {
                assert!(editor.dusting());
                assert!(editor.dialogs.message().is_none(), "{}", editor.status);
                let summary = editor.document.export.summary.as_ref().unwrap();
                assert_eq!((summary.exported(), summary.failed()), (1, 0));
                assert_eq!(request.photos[0].recipe, recipe);
            });
            let image = exr::prelude::read_first_flat_layer_from_file(&path).unwrap();
            let index = image.layer_data.size.0 * 24 + 24;
            for (channel, name) in ["R", "G", "B"].into_iter().enumerate() {
                let values = image
                    .layer_data
                    .channel_data
                    .list
                    .iter()
                    .find(|c| c.name.to_string() == name)
                    .unwrap();
                let exr::prelude::FlatSamples::F32(values) = &values.sample_data else {
                    panic!("full-float EXR")
                };
                assert!(
                    (f64::from(values[index]) - expected[channel]).abs()
                        <= crate::working_precision::allowance(expected[channel]),
                    "{name}: {} != {}",
                    values[index],
                    expected[channel]
                );
            }
        });
    }
}
