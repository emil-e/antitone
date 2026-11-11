use super::*;

#[test]
fn save_declines_unfinished_region_consumers_without_retiring_them_or_queueing_a_write() {
    for analyze in [false, true] {
        let (scheduler, _) = observed();
        let mut editor = fixture(scheduler.clone());
        let metadata = supplied(&mut editor);
        editor.document.spatial = crate::spatial_metadata::Store::default();
        let reply = programmed(&mut editor);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-queued.antitone");
        with_application(editor, |app| {
            let accepted = app.enter(|editor, cx| {
                if analyze {
                    editor.request_analysis(cx);
                } else {
                    editor.review_crop_proposals(
                        editor.document.project.order.clone(),
                        "Whole roll",
                        cx,
                    );
                }
                let accepted = editor.document.project.clone();
                editor.save_project(path.clone(), Save::New, None, cx);
                assert!(!editor.document.write.pending() && !editor.files.pending());
                assert!(editor.document.spatial_preparing.is_some());
                accepted
            });
            app.pump();
            assert!(!path.exists());
            reply.send(Ok(Some(metadata.clone()))).unwrap();
            app.run_until(LIMIT, |editor| {
                editor.document.spatial_preparing.is_none()
                    && if analyze {
                        editor.document.analysis_review().is_some()
                    } else {
                        editor.document.auto_crop.reviewing()
                    }
            });
            app.read(|editor| {
                assert_eq!(editor.document.project, accepted);
                assert_eq!(*editor.document.spatial.ready(), metadata);
                assert!(!editor.document.write.pending() && !editor.dialogs.is_open());
            });
            assert!(
                !path.exists(),
                "a refused Save must not replay when the consumer becomes ready"
            );
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn save_after_finishing_the_consumer_reports_real_file_failure_without_losing_accepted_state() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let metadata = supplied(&mut editor);
    editor.document.spatial = crate::spatial_metadata::Store::default();
    let reply = programmed(&mut editor);
    let accepted = editor.document.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("occupied.antitone");
    std::fs::write(&path, b"existing destination").unwrap();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
            editor.save_project(path.clone(), Save::New, None, cx);
            assert!(!editor.document.write.pending());
        });
        app.pump();
        assert!(app.read(|editor| !editor.dialogs.is_open()));
        reply.send(Ok(Some(metadata.clone()))).unwrap();
        app.run_until(LIMIT, |editor| editor.document.auto_crop.reviewing());
        app.enter(|editor, cx| {
            editor.finish_auto_crop(false);
            editor.save_project(path.clone(), Save::New, None, cx);
            assert!(editor.document.write.pending());
        });
        app.run_until(LIMIT, |editor| !editor.document.write.pending());
        app.read(|editor| {
            assert!(editor.dialogs.is_open());
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                "Could not save project"
            );
            assert!(!editor.document.auto_crop.active());
            assert_eq!(editor.document.project, accepted);
            assert_eq!(*editor.document.spatial.ready(), metadata);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    });
    scheduler.close_and_wait().unwrap();
}
