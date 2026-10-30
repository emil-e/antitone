use super::*;

#[test]
fn regions_ready_before_a_write_completes_resume_either_original_consumer_after_save() {
    for analyze in [false, true] {
        let (scheduler, _) = observed();
        let mut editor = fixture(scheduler.clone());
        let metadata = supplied(&mut editor);
        editor.document.spatial = crate::spatial_metadata::Store::default();
        let reply = programmed(&mut editor);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.antitone");
        // Only the physical write is parked: the programmed preparation uses its normal async producer.
        let write = crate::background::testing::pause();
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                if analyze {
                    editor.request_analysis(cx);
                } else {
                    editor.review_crop_proposals(
                        editor.document.project.order.clone(),
                        "Whole roll",
                        cx,
                    );
                }
                editor.save_project(path.clone(), Save::New, None, cx);
            });
            app.pump();
            assert!(app.read(|editor| editor.document_pending.is_some()));
            reply.send(Ok(Some(metadata.clone()))).unwrap();
            app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
            app.read(|editor| {
                assert!(editor.document_pending.is_some());
                assert!(editor.document.auto_crop_edit().is_none());
                assert!(!editor.analysis_preparing());
                assert_eq!(*editor.document.spatial.ready(), metadata);
            });
            drop(write);
            app.run_until(LIMIT, |editor| {
                editor.document_pending.is_none()
                    && !editor.busy
                    && if analyze {
                        editor.document.analysis_review().is_some()
                    } else {
                        editor.document.auto_crop_edit().is_some()
                    }
            });
            app.read(|editor| {
                assert!(editor.failure.is_none(), "{}", editor.status);
                assert_eq!(*editor.document.spatial.ready(), metadata);
                assert!(
                    editor
                        .document
                        .project
                        .sources
                        .iter()
                        .all(|source| source.crop.is_none())
                );
                assert!(editor.document.state.project.analysis_regions.is_empty());
                assert!(!editor.document.dirty());
            });
            let (_, saved) = Project::load_with_spatial_metadata(&path).unwrap();
            assert!(saved.groups.is_empty());
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn failed_overlapping_save_keeps_the_consumer_until_failure_acknowledgement() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let metadata = supplied(&mut editor);
    editor.document.spatial = crate::spatial_metadata::Store::default();
    let reply = programmed(&mut editor);
    let accepted = editor.document.state.project.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.antitone");
    std::fs::write(&path, b"existing destination").unwrap();
    let write = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
            editor.save_project(path.clone(), Save::New, None, cx);
        });
        app.pump();
        reply.send(Ok(Some(metadata.clone()))).unwrap();
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        drop(write);
        app.run_until(LIMIT, |editor| editor.document_pending.is_none());
        app.read(|editor| {
            assert!(editor.failure.is_some());
            assert!(editor.document.auto_crop_operation().is_some());
            assert!(editor.document.auto_crop_edit().is_none());
        });
        let context = egui::Context::default();
        crate::editor::tests::app_frame(app, &context, vec![]);
        crate::editor::tests::app_frame(
            app,
            &context,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        app.run_until(LIMIT, |editor| editor.document.auto_crop_edit().is_some());
        app.read(|editor| {
            assert!(editor.failure.is_none());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(*editor.document.spatial.ready(), metadata);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    });
    scheduler.close_and_wait().unwrap();
}
