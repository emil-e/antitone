use super::*;
use crate::editor::analysis_review;

#[test]
fn a_completed_reply_cannot_publish_after_supporting_geometry_changes() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let peer = editor.document.project.order[1];
    let metadata = supplied(&mut editor);
    editor.roll_selection = Default::default();
    editor
        .roll_selection
        .reconcile(&editor.document.project.order, editor.current_source);
    let accepted = editor.document.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.request_auto_crop(auto_crop::Target::Selected, cx);
            let request = editor.document.spatial_preparing.as_ref().unwrap().id;
            editor
                .document
                .project
                .source_mut(peer)
                .unwrap()
                .development
                .active_area[2] -= 1;
            editor.spatial_result(request, Ok(Some(metadata)), cx);
            assert!(editor.document.spatial_preparing.is_none());
            assert!(
                editor.document.auto_crop_edit.is_none(),
                "stale peer dimensions must not reach a consumer"
            );
            assert_eq!(
                editor
                    .document
                    .project
                    .source(accepted.order[0])
                    .unwrap()
                    .crop,
                accepted.sources[0].crop
            );
            assert!(
                editor
                    .document
                    .capture()
                    .ready_spatial_metadata()
                    .groups
                    .is_empty()
            );
        });
        app.pump();
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn analysis_verification_checks_hidden_dimension_supporters_not_only_visible_sampling_files() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let peer = editor.document.project.order[1];
    supplied(&mut editor);
    editor.document.project.order.truncate(1);
    let mut request = editor.analysis_request().unwrap();
    assert!(analysis_review::verify(&request).is_ok());
    request
        .document
        .project
        .source_mut(peer)
        .unwrap()
        .development
        .active_area[2] -= 1;
    assert!(
        analysis_review::verify(&request).is_err(),
        "a hidden supporting observation is still a real dependency"
    );
    assert!(
        editor
            .document
            .project
            .sources
            .iter()
            .all(|source| source.crop.is_none())
    );
    scheduler.close_and_wait().unwrap();
}

#[test]
fn save_as_rebased_relative_locators_do_not_retire_a_completed_shared_result() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    // Only the locator base is hypothetical; all actual source files remain committed/read-only.
    editor.document.path = Some(root.join("origin.antitone"));
    for source in &mut editor.document.project.sources {
        source.path = source.path.file_name().unwrap().into();
    }
    editor.document.capture_source_files().unwrap();
    let metadata = supplied(&mut editor);
    editor.document.spatial = crate::spatial_metadata::Store::default();
    // Keep the saved project on the sources' volume: a relative locator cannot span volumes,
    // and the system temporary directory may live on another drive than the checkout.
    let dir = tempfile::Builder::new()
        .prefix("antitone-moved-")
        .tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    let path = dir.path().join("moved.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.request_auto_crop(auto_crop::Target::Roll, cx);
            let request = editor.document.spatial_preparing.as_ref().unwrap().id;
            let written = editor.document.capture().save(&path, Save::New).unwrap();
            editor.document.accept_save(written);
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|source| source.path.is_relative())
            );
            editor.spatial_result(request, Ok(Some(metadata.clone())), cx);
            assert_eq!(*editor.document.spatial.ready(), metadata);
            assert!(
                editor.document.auto_crop_edit.is_some(),
                "{}",
                editor.status
            );
            assert!(!editor.document.dirty());
        });
        app.pump();
    });
    scheduler.close_and_wait().unwrap();
}
