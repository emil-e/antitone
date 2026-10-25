use super::*;
use crate::platform::menu::Action;

fn fixture_editor(test: impl FnOnce(Harness<'_, Editor>, tempfile::TempDir)) {
    let directory = tempfile::tempdir().unwrap();
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        harness.enter(|editor, cx| {
            editor.add_raws(vec![fixture("patches.dng"), fixture("roll-0.dng")], cx)
        });
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(0.8)));
            editor.set_dmax(Some(DVec3::splat(0.02)));
            editor.target = OutputTarget::DisplayP3;
            editor.crop = Some(Crop {
                center: [120., 100.],
                size: [16, 16],
                angle: 0.,
            });
            editor.sync_document();
            for id in editor.document.project.order.clone() {
                editor.document.project.source_mut(id).unwrap().crop = editor.crop;
            }
            let (store, _) =
                preferences::Store::open(directory.path().join("preferences.json")).unwrap();
            editor.preference_store = Some(store);
            editor.preferences.export_folder = Some(directory.path().to_owned());
        });
        test(harness, directory);
    });
}

fn open(harness: &mut Harness<'_, Editor>) {
    harness.update(|editor| host::App::menu_action(editor, Action::Export));
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn choose(harness: &mut Harness<'_, Editor>, name: &str, value: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, name)
        .click();
    harness.step();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(value).click();
    harness.step();
    harness.try_run_realtime().unwrap();
}

#[test]
fn dialog_preserves_project_state_and_exports_all_with_its_own_target() {
    fixture_editor(|mut harness, directory| {
        let accepted = harness.read(|editor| editor.document.project.clone());
        let active = harness.read(|editor| editor.current_source);
        open(&mut harness);
        harness.read(|editor| {
            assert!(!editor.application_menu_state().enabled(Action::New));
            assert_eq!(
                editor.export_dialog.as_ref().unwrap().target,
                OutputTarget::DisplayP3
            );
        });
        choose(&mut harness, "Format", "EXR");
        assert!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Target")
                .accesskit_node()
                .is_disabled()
        );
        choose(&mut harness, "Precision", "Full float · 32-bit");
        let png = harness.render().unwrap();
        save_capture(&png, "export-dialog-exr-offscreen");
        choose(&mut harness, "Format", "TIFF");
        harness.read(|editor| {
            assert_eq!(
                editor.export_dialog.as_ref().unwrap().target,
                OutputTarget::DisplayP3
            )
        });
        choose(&mut harness, "Target", "sRGB");
        harness.get_by_label("All photos (2)").click();
        harness.step();
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            assert_eq!(editor.document.project, accepted);
            assert_eq!(editor.target, OutputTarget::DisplayP3);
        });
        let png = harness.render().unwrap();
        save_capture(&png, "export-dialog-tiff-offscreen");
        harness.get_by_label("Export 2 photos").click();
        harness.step();
        wait_for(&mut harness, |editor| {
            editor.export_operation.pending.is_none()
        });
        harness.read(|editor| {
            assert_eq!(
                editor.export_summary.as_ref().unwrap().exported(),
                2,
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project, accepted);
            assert_eq!(editor.current_source, active);
            assert_eq!(editor.target, OutputTarget::DisplayP3);
        });
        for name in ["patches.tif", "roll-0.tif"] {
            let mut decoder = tiff::decoder::Decoder::new(
                std::fs::File::open(directory.path().join(name)).unwrap(),
            )
            .unwrap();
            assert_eq!(decoder.dimensions().unwrap(), (16, 16));
            assert_eq!(
                decoder
                    .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                    .unwrap(),
                crate::profiles::icc(OutputTarget::Srgb)
            );
        }
        assert!(
            !harness
                .get_by_label("Show in Finder")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("OK").click();
        harness.step();
        open(&mut harness);
        harness.read(|editor| {
            let draft = editor.export_dialog.as_ref().unwrap();
            assert_eq!(draft.target, OutputTarget::DisplayP3);
            assert_eq!(draft.folder, directory.path());
        });
        harness.key_press(egui::Key::Escape);
        harness.try_run_realtime().unwrap();
        harness.read(|editor| assert!(editor.export_dialog.is_none()));
        let restored = preferences::Store::open(directory.path().join("preferences.json"))
            .unwrap()
            .1;
        assert_eq!(restored.export_folder.as_deref(), Some(directory.path()));
    });
}

#[test]
fn selected_exr_export_can_be_cancelled_and_retried_without_exporting_unselected_photos() {
    fixture_editor(|mut harness, directory| {
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("bare-light.dng")], cx));
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness
            .get_by_label("roll-0.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        harness.step();
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            assert_eq!(editor.export_ids(export_batch::Scope::Selected).len(), 2);
            assert_eq!(editor.export_ids(export_batch::Scope::All).len(), 3);
        });
        let accepted = harness.read(|editor| editor.document.project.clone());
        open(&mut harness);
        choose(&mut harness, "Format", "EXR");
        choose(&mut harness, "Precision", "Full float · 32-bit");
        let jobs = harness.read(|editor| editor.workflow_jobs.clone());
        let admission = jobs.hold();
        harness.get_by_label("Export 2 photos").click();
        harness.step();
        harness.update(|editor| editor.tasks.elapse(Duration::from_secs(1)));
        harness.try_run_realtime().unwrap();
        let progress =
            harness.get_by_role_and_label(egui::accesskit::Role::ProgressIndicator, "Exporting…");
        assert!(!progress.accesskit_node().is_disabled());
        let png = harness.render().unwrap();
        save_capture(&png, "export-progress-modal-offscreen");
        harness.get_by_label("Cancel export").click();
        harness.step();
        harness.read(|editor| {
            assert!(editor.export_operation.pending.is_some());
            assert!(
                editor
                    .export_operation
                    .cancelled
                    .as_ref()
                    .unwrap()
                    .is_cancelled()
            );
            assert!(
                editor.tasks.view().unwrap().cancel.is_none(),
                "cancellation stays pending until service acknowledgement"
            );
        });
        harness.step();
        let png = harness.render().unwrap();
        save_capture(&png, "export-cancelling-modal-offscreen");
        drop(admission);
        wait_for(&mut harness, |editor| {
            editor.export_operation.pending.is_none()
        });
        harness.read(|editor| {
            assert!(editor.failure.is_none() && !editor.busy && editor.tasks.view().is_none());
            assert_eq!(editor.document.project, accepted);
        });
        assert!(!directory.path().join("patches.exr").exists());
        assert!(!directory.path().join("roll-0.exr").exists());
        open(&mut harness);
        choose(&mut harness, "Format", "EXR");
        choose(&mut harness, "Precision", "Full float · 32-bit");
        harness.get_by_label("Export 2 photos").click();
        harness.step();
        wait_for(&mut harness, |editor| {
            editor.export_operation.pending.is_none()
        });
        harness.read(|editor| {
            assert_eq!(
                editor.export_summary.as_ref().unwrap().exported(),
                2,
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project, accepted);
        });
        for name in ["patches.exr", "roll-0.exr"] {
            let image =
                exr::prelude::read_first_flat_layer_from_file(directory.path().join(name)).unwrap();
            for channel in &image.layer_data.channel_data.list {
                assert!(matches!(
                    channel.sample_data,
                    exr::prelude::FlatSamples::F32(_)
                ));
            }
        }
        assert!(!directory.path().join("bare-light.exr").exists());
    });
}

#[test]
fn opening_export_commits_pending_photo_numeric_text_before_blocking_the_workspace() {
    fixture_editor(|mut harness, _| {
        stage(&mut harness, "Photo");
        let role = egui::accesskit::Role::SpinButton;
        harness
            .get_by_role_and_label(role, "Exposure")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_role_and_label(role, "Exposure").click();
        harness.step();
        harness.event(egui::Event::Text("1.25".into()));
        harness.step();
        open(&mut harness);
        harness.read(|editor| {
            assert_eq!(editor.photo.exposure, 1.25);
            assert_eq!(
                editor
                    .document
                    .project
                    .source(editor.current_source.unwrap())
                    .unwrap()
                    .photo
                    .exposure,
                1.25
            );
        });
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.photo.exposure, 1.25));
    });
}

#[test]
fn destination_paths_share_a_left_edge_and_choose_stays_on_the_right() {
    fixture_editor(|mut harness, directory| {
        open(&mut harness);
        harness.hover_at(egui::pos2(-50., -50.));
        for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
            harness.set_size(size);
            let mut starts = Vec::new();
            for folder in [
                directory.path().to_owned(),
                directory.path().join("Photographs"),
            ] {
                harness.update(|editor| {
                    editor.export_dialog.as_mut().unwrap().folder = folder.clone()
                });
                harness.try_run_realtime().unwrap();
                let text = folder.display().to_string();
                let path = harness
                    .output()
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text_shape) if text_shape.galley.job.text == text => {
                            Some(text_shape)
                        }
                        _ => None,
                    })
                    .expect("destination text must be painted");
                let choose = harness
                    .get_by_role_and_label(egui::accesskit::Role::Button, "Choose…")
                    .rect();
                assert!(
                    path.pos.x + path.galley.size().x < choose.left(),
                    "fixture paths must fit without truncation"
                );
                starts.push(path.pos.x);
                let right = harness
                    .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Format")
                    .rect()
                    .right();
                assert!(
                    (choose.right() - right).abs() <= 1.,
                    "Choose must align with the form's right edge"
                );
            }
            assert!(
                (starts[0] - starts[1]).abs() <= 1.,
                "path length must not move the text's left edge: {starts:?}"
            );
            let png = harness.render().unwrap();
            save_capture(
                &png,
                &format!("export-destination-{}-offscreen", size.x as u32),
            );
        }
    });
}

#[test]
fn narrow_dialog_can_be_cancelled_without_exporting_or_changing_the_document() {
    fixture_editor(|mut harness, directory| {
        let accepted = harness.read(|editor| editor.document.project.clone());
        harness.set_size(egui::vec2(800., 600.));
        open(&mut harness);
        let cancel = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel");
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.))
                .contains_rect(cancel.rect()),
            "dialog actions must remain reachable"
        );
        let png = harness.render().unwrap();
        save_capture(&png, "export-dialog-narrow-offscreen");
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            assert!(editor.export_dialog.is_none() && editor.export_operation.pending.is_none());
            assert_eq!(editor.document.project, accepted);
        });
        assert!(!directory.path().join("patches.tif").exists());
    });
}
