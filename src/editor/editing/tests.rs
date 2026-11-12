use super::*;

#[test]
fn document_edits_share_admission_and_publish_registered_source_metadata_with_history() {
    let mut editor = editor();
    let before = editor.document.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reference.raw");
    std::fs::write(&path, b"file identity fixture").unwrap();
    let expected = crate::source::FileKey::new(&path);
    let mut edit = editor.edit_document("Document edit").unwrap();
    assert!(editor.edit_document("Another document").is_err());
    assert!(editor.edit_project("Project edit").is_err());
    assert!(editor.edit_photo("Photo edit").is_err());
    let reference = edit.update(|document| {
        let mut record = document
            .project
            .source(document.project.order[0])
            .unwrap()
            .clone();
        record.path = path.clone();
        let reference = document.register(record).unwrap();
        let record = document.project.source(reference).unwrap();
        let map = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                record.development.active_area,
                [2, 2],
                vec![DVec3::ONE; 4],
            )
            .unwrap(),
        );
        let field = crate::project::FlatFieldSettings::new(record.path.clone(), map);
        document.project.set_flat_field(Some(field)).unwrap();
        reference
    });
    assert_eq!(editor.document.project, before);
    std::fs::remove_file(&path).unwrap();
    editor.commit_document(edit).unwrap();
    assert!(editor.document.project.order.contains(&reference));
    assert_eq!(
        editor.document.file_key(&path),
        expected,
        "commit preserves captured metadata rather than recapturing a missing file"
    );
    assert_eq!(
        editor.history_label(history::Direction::Undo),
        Some("Document edit")
    );
    assert!(!editor.editing());
    assert!(editor.edit_project("Next project edit").is_ok());
}

#[test]
fn document_readers_do_not_retain_mutation_rights_and_other_edits_deny_document_acquisition() {
    let mut editor = editor();
    let project = editor.edit_project("Project edit").unwrap();
    assert!(editor.edit_document("Document edit").is_err());
    drop(project);
    let photo = editor.edit_photo("Photo edit").unwrap();
    assert!(editor.edit_document("Document edit").is_err());
    drop(photo);
    let mut document = editor.edit_document("Abandoned").unwrap();
    document.update(|candidate| candidate.project.roll.exposure = 1.);
    let reader = document.preview().snapshot().unwrap();
    drop(document);
    assert_eq!(reader.project.roll.exposure, 1.);
    assert_eq!(editor.document.project.roll.exposure, 0.);
    assert!(!editor.editing());
    assert!(editor.history_label(history::Direction::Undo).is_none());
    assert!(editor.edit_document("Available again").is_ok());
}

#[test]
fn retiring_document_edits_rejects_old_publication_without_retiring_the_new_owner() {
    let mut editor = editor();
    let before = editor.document.project.clone();
    let mut old = editor.edit_document("Old document edit").unwrap();
    old.update(|candidate| candidate.project.roll.exposure = 1.);
    editor.document.retire_edits();
    let next = editor.edit_project("Next edit").unwrap();
    assert!(editor.commit_document(old).is_err());
    assert!(next.preview().active());
    assert_eq!(editor.document.project, before);
    assert!(editor.history_label(history::Direction::Undo).is_none());
    editor.commit_project(next).unwrap();
}

#[test]
fn readers_do_not_retain_an_abandoned_edit() {
    let mut edit = Edit::from_snapshot(Arc::new(vec![1, 2]), "Edit", None);
    let preview = edit.preview();
    let snapshot = preview.snapshot().unwrap();
    edit.update(|values| values.push(3));
    assert_eq!(*preview.snapshot().unwrap(), vec![1, 2, 3]);
    assert_eq!(*snapshot, vec![1, 2]);
    drop(edit);
    assert!(!preview.active());
    assert!(preview.snapshot().is_none());
    assert_eq!(*snapshot, vec![1, 2]);
}

#[test]
fn consuming_the_owner_retires_its_preview_without_invalidating_readers() {
    let edit = Edit::from_snapshot(Arc::new(7), "Edit", None);
    let preview = edit.preview();
    let reader = preview.snapshot().unwrap();
    let accepted = edit.into_snapshot();
    assert!(!preview.active());
    assert_eq!(*reader, 7);
    assert_eq!(*accepted, 7);
}

#[test]
fn an_existing_owner_rejects_another_acquisition_until_commit_or_drop() {
    let mut editor = crate::editor::tests::fixture();
    let original = editor.document.project.clone();
    let mut first = editor.edit_project("Exposure").unwrap();
    first.update(|project| project.roll.exposure = 1.25);
    assert_eq!(editor.document.project, original);
    assert_eq!(editor.document.project_snapshot().roll.exposure, 1.25);
    assert!(editor.edit_project("Target").is_err());
    assert!(editor.edit_photo("Photo").is_err());
    assert!(first.preview().active());
    let reader = first.preview();
    editor.commit_project(first).unwrap();
    assert!(!reader.active());
    let mut second = editor.edit_project("Target").unwrap();
    second.update(|project| project.output.tiff = crate::project::TiffTarget::Srgb);
    editor.commit_project(second).unwrap();
    assert_eq!(editor.document.project.roll.exposure, 1.25);
    assert_eq!(
        editor.document.project.output.tiff,
        crate::project::TiffTarget::Srgb
    );
    assert_eq!(
        editor.history_label(history::Direction::Undo),
        Some("Target")
    );
    let abandoned = editor.edit_project("Abandoned").unwrap();
    drop(abandoned);
    assert!(editor.edit_project("Available again").is_ok());
}

#[test]
fn unchanged_and_abandoned_edits_preserve_accepted_state_and_history() {
    let mut editor = crate::editor::tests::fixture();
    let original = editor.document.project.clone();
    let unchanged = editor.edit_project("Unchanged").unwrap();
    editor.commit_project(unchanged).unwrap();
    let mut abandoned = editor.edit_project("Abandoned").unwrap();
    abandoned.update(|project| project.roll.exposure = 0.75);
    let reader = abandoned.preview().snapshot().unwrap();
    drop(abandoned);
    assert_eq!(reader.roll.exposure, 0.75);
    assert_eq!(editor.document.project_snapshot(), original);
    assert_eq!(editor.history_label(history::Direction::Undo), None);
    assert!(!editor.editing());
}

#[test]
fn a_retired_photo_owner_cannot_publish_into_a_new_scope() {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    let original = editor.document.project.clone();
    let mut old = editor.edit_photo("Old photo").unwrap();
    editor.replace_photo_scope(editor.document.visit.view + 1);
    old.update(|project| project.roll.exposure = 0.75);
    let current = editor.edit_photo("Current photo").unwrap();
    assert!(editor.commit_project(old).is_err());
    assert!(current.preview().active());
    assert_eq!(editor.document.project, original);
    assert_eq!(editor.document.project_snapshot(), original);
    assert_eq!(editor.history_label(history::Direction::Undo), None);
    editor.commit_project(current).unwrap();
}

#[test]
fn editing_a_version_preserves_history_and_reader_versions_without_changing_serialization() {
    let mut current = ProjectVersion::default();
    let history = current.clone();
    let reader = current.snapshot();
    current.roll.exposure = 1.25;
    assert_eq!(history.roll.exposure, 0.);
    assert_eq!(reader.roll.exposure, 0.);
    assert_eq!(current.roll.exposure, 1.25);
    assert_eq!(
        serde_json::to_value(&current).unwrap(),
        serde_json::to_value(current.as_ref()).unwrap()
    );
    let reopened: ProjectVersion =
        serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
    assert_eq!(current, reopened);
}

#[test]
fn roll_controls_reject_nonfinite_values_and_output_controls_accept_target_edits_without_a_camera()
{
    for field in 0..3 {
        let editor = crate::editor::tests::fixture();
        assert!(editor.document.project.sources.is_empty());
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.output_control(
                    ui::ControlResponse::action(egui::Id::new("target"), true),
                    |output| output.tiff = crate::project::TiffTarget::Srgb,
                );
                assert_eq!(editor.document.target(), OutputTarget::Srgb);
                let accepted = editor.document.project.clone();
                editor.roll_control(
                    ui::ControlResponse::action(egui::Id::new("invalid"), true),
                    cx,
                    |project| match field {
                        0 => project.roll.exposure = f64::NAN,
                        1 => project.roll.wb.warmth = f64::NAN,
                        _ => {
                            let InputSettings::Negative { gamma, .. } = &mut project.input else {
                                panic!("negative fixture");
                            };
                            *gamma = f64::NAN;
                        }
                    },
                );
                assert!(editor.status.contains("rejected"), "{}", editor.status);
                assert_eq!(editor.document.project, accepted);
                assert!(!editor.editing());
                assert_eq!(
                    editor.history_label(history::Direction::Undo),
                    Some("Roll settings")
                );
            });
        });
    }
}

#[test]
fn rendering_controls_validate_their_own_texture_values_before_publishing() {
    for grain in [false, true] {
        let mut editor = crate::editor::tests::fixture();
        let response = |id| ui::ControlResponse::action(egui::Id::new(id), true);
        editor.output_control(response("valid"), |output| output.grain.amount = 0.12);
        assert_eq!(editor.document.project.output.grain.amount, 0.12);
        let accepted = editor.document.project.clone();
        editor.output_control(response("invalid"), |output| {
            if grain {
                output.grain.amount = f64::NAN;
            } else {
                output.halation.strength = f64::NAN;
            }
        });
        assert!(editor.status.contains("rejected"), "{}", editor.status);
        assert_eq!(editor.document.project, accepted);
        assert!(!editor.editing());
    }
}

fn editor() -> Editor {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::with_camera_basis(&mut editor);
    editor.document.visit.current_source = editor.document.project.order.first().copied();
    assert!(
        editor.document.visit.current_source.is_some(),
        "fixture has an ordered source"
    );
    editor.document.source = false;
    editor.refresh_adjustment_layer_selection();
    editor
}

#[test]
fn project_draft_drives_recipes_without_changing_accepted_or_saved_values() {
    let mut editor = editor();
    let accepted = editor.document.project.clone();
    let mut edit = editor.edit_project("Preview").unwrap();
    edit.update(|project| {
        project.roll.exposure = 1.;
        project.output.grain.amount = 0.12;
    });
    let current = editor.document.project_snapshot();
    let recipe = editor
        .positive_recipe(current.input, current.roll, editor.photo_settings())
        .unwrap();
    assert_eq!(recipe.roll.exposure, 1.);
    assert_eq!(recipe.grain.amount, 0.12);
    assert_eq!(editor.document.state.project, accepted);
    assert_eq!(editor.document.capture().project, accepted);
    let observer = edit.preview();
    drop(edit);
    assert!(!observer.active());
    assert!(!editor.editing());
    let current = editor.document.project_snapshot();
    let recipe = editor
        .positive_recipe(current.input, current.roll, editor.photo_settings())
        .unwrap();
    assert_eq!(recipe.roll, accepted.roll);
    assert_eq!(recipe.grain, accepted.output.grain);
}

#[test]
fn photo_project_draft_contains_layer_strength_and_grade_and_retires_with_the_visit() {
    let mut editor = editor();
    let accepted = editor.document.project.clone();
    let source = editor.document.visit.current_source.unwrap();
    let mut edit = editor.edit_photo("Photo draft").unwrap();
    edit.update(|project| {
        let record = &mut project
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers[0];
        record.adjustment.photo.exposure = 0.75;
        record.adjustment.strength = 0.4;
    });
    let effective = editor.document.project_snapshot();
    let record = &effective
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0];
    assert_eq!(record.adjustment.photo.exposure, 0.75);
    assert_eq!(record.adjustment.strength, 0.4);
    assert_eq!(editor.photo_settings().exposure, 0.75);
    assert_eq!(editor.document.project, accepted);
    assert!(!editor.file_available());
    editor.replace_photo_scope(editor.document.visit.view + 1);
    assert!(!editor.editing());
    assert!(editor.commit_project(edit).is_err());
    assert_eq!(editor.photo_settings().exposure, 0.);
    assert_eq!(editor.document.project_snapshot(), accepted);
    assert_eq!(editor.history_label(history::Direction::Undo), None);
}

#[test]
fn late_commit_after_install_leaves_accepted_state_unchanged() {
    let mut editor = editor();
    let mut retired = editor.edit_project("Retired").unwrap();
    retired.update(|project| project.roll.exposure = 1.);
    let mut replacement = editor.document.capture();
    replacement.project.roll.exposure = -0.5;
    editor.document.install(replacement);
    assert!(!editor.editing());
    assert!(editor.commit_project(retired).is_err());
    assert_eq!(editor.document.project.roll.exposure, -0.5);
    assert_eq!(editor.history_label(history::Direction::Undo), None);
}

#[test]
fn save_and_export_are_unavailable_until_the_edit_is_committed_or_dropped() {
    let mut editor = editor();
    use crate::platform::menu::Action;
    assert!(editor.file_available());
    let edit = editor.edit_project("Unfinished").unwrap();
    for action in [Action::Save, Action::SaveAs, Action::Export] {
        assert!(!editor.application_menu_state().enabled(action));
    }
    let directory = tempfile::tempdir().unwrap();
    let draft = export::Draft::new(OutputTarget::DisplayP3, directory.path().to_owned());
    assert!(
        editor
            .capture_export(&draft)
            .err()
            .expect("draft blocks export capture")
            .to_string()
            .contains("current edit")
    );
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
    drop(edit);
    assert!(editor.file_available());
    for action in [Action::Save, Action::SaveAs, Action::Export] {
        assert!(editor.application_menu_state().enabled(action));
    }
}
