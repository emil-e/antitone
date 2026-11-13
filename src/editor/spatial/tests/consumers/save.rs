use super::*;

#[test]
fn save_command_respects_shared_preparation_input_without_retiring_consumers_or_queueing_a_write() {
    for analyze in [false, true] {
        let (scheduler, _) = observed();
        let mut editor = fixture(scheduler.clone());
        let metadata = supplied(&mut editor);
        editor.document.spatial = crate::spatial_metadata::Store::default();
        let reply = programmed(&mut editor);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-queued.antitone");
        editor.document.path = Some(path.clone());
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
                assert!(
                    !editor
                        .application_menu_state()
                        .enabled(crate::platform::menu::Action::Save)
                );
                editor.file_action(crate::editor::files::FileAction::Save, cx);
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
    editor.document.path = Some(path.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
            editor.file_action(crate::editor::files::FileAction::Save, cx);
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
