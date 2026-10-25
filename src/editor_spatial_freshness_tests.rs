use super::*;
use crate::editor::analysis_review;

fn copied_supporter(editor: &mut Editor, dir: &std::path::Path) -> std::path::PathBuf {
    let id = editor.document.project.order[1];
    let path = dir.join("supporter.dng");
    std::fs::copy(editor.document.source_path(id).unwrap(), &path).unwrap();
    editor.document.project.source_mut(id).unwrap().path = path.clone();
    path
}

fn revise(path: &std::path::Path) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes.extend_from_slice(b"a fresh revision without changing declared sensor data");
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_completed_reply_waiting_for_the_ui_cannot_publish_after_a_supporting_peer_changes() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let dir = tempfile::tempdir().unwrap();
    let peer = copied_supporter(&mut editor, dir.path());
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
            // Supply the completed old capture before the deferred task starts. Only the
            // non-target peer changes, so target-only proposal freshness cannot catch this.
            revise(&peer);
            editor.spatial_result(request, Ok(Some(metadata)), cx);
            assert!(editor.document.spatial_preparing.is_none());
            assert!(
                editor.document.auto_crop_edit.is_none(),
                "stale peer dimensions must not reach a consumer"
            );
            assert_eq!(editor.document.project, accepted);
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
    let dir = tempfile::tempdir().unwrap();
    let peer = copied_supporter(&mut editor, dir.path());
    supplied(&mut editor);
    editor.document.project.order.truncate(1);
    let request = editor.analysis_request().unwrap();
    assert!(analysis_review::verify(&request).is_ok());
    revise(&peer);
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
