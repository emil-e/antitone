use super::*;

#[test]
fn snapshot_failures_leave_search_publication_and_existing_destinations_intact() {
    for exporting in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("occupied.antitone");
        std::fs::write(&path, b"existing destination").unwrap();
        let mut editor = authored_editor();
        if exporting {
            editor.export_services = None;
        }
        with_search(editor, |app, gate| {
            let stroke = app.enter(|editor, cx| {
                editor.begin_dust_cleanup();
                queue(editor, [100, 110], cx)
            });
            let call = gate.next(app);
            if exporting {
                open_form(app, directory.path());
                confirm_form(app);
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.export.operation.is_none()
                });
            } else {
                app.enter(|editor, cx| {
                    editor.save_project(path.clone(), crate::project::Save::New, None, cx)
                });
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.write.pending()
                });
            }
            app.read(|editor| {
                assert!(editor.dusting() && editor.dust_search_pending());
                assert_eq!(
                    editor.dialogs.message().unwrap().title,
                    if exporting {
                        "Could not export image"
                    } else {
                        "Could not save project"
                    }
                );
            });
            assert!(!call.cancelled.is_cancelled());
            call.complete(Ok(SearchOutcome::Candidate([20, 0])));
            settled(app);
            app.read(|editor| {
                assert!(editor.dusting() && editor.dialogs.is_open());
                assert_eq!(
                    editor.dust_repairs(stroke.source).unwrap().repairs().len(),
                    2
                );
                assert!(editor.document.dirty());
            });
            assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        });
    }
}

#[test]
fn cancelling_export_does_not_cancel_an_existing_dust_search_or_close_its_tool() {
    let directory = tempfile::tempdir().unwrap();
    with_search(authored_editor(), |app, gate| {
        let stroke = app.enter(|editor, cx| {
            editor.begin_dust_cleanup();
            queue(editor, [100, 110], cx)
        });
        let call = gate.next(app);
        open_form(app, directory.path());
        let context = egui::Context::default();
        app.enter(|editor, cx| {
            frame(editor, &context, vec![], cx);
            frame(
                editor,
                &context,
                crate::editor::ui::test_support::key(egui::Key::Escape),
                cx,
            );
            assert!(editor.document.export.form.is_none());
            assert!(editor.document.export.operation.is_none());
            assert!(editor.dusting() && editor.dust_search_pending());
        });
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|editor| {
            assert!(editor.dusting());
            assert_eq!(
                editor.dust_repairs(stroke.source).unwrap().repairs().len(),
                2
            );
        });
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    });
}
