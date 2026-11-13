use super::*;

#[test]
fn history_interpretation_change_retires_the_old_stage_reference_scope() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        app.enter(|e, cx| e.request_input(crate::processing::InputSettings::Digital, cx));
        app.update(|e| {
            if e.document.input_change.is_some() {
                e.dialogs.answer_for_test("Switch stage");
            }
        });
        wait(app);
        assert!(app.read(|e| !e.document.project.input.is_negative()));
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        wait(app);
        assert!(app.read(|e| e.document.project.input.is_negative()));
        begin(app, "bare-light.dng");
        wait(app);
        app.read(|e| assert!(e.document.flat_edit().unwrap().view.ready.is_some()));
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        wait(app);
        let restored = app.read(|e| {
            assert!(!e.document.project.input.is_negative());
            assert!(e.document.flat_edit().is_none());
            assert!(e.current_tool().is_none());
            e.document.state.project.clone()
        });
        apply(app);
        app.pump();
        app.read(|e| assert_eq!(e.document.state.project, restored));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn history_keeps_the_prepared_reference_and_apply_uses_the_restored_document() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let before = app.update(|e| {
            let before = e.document.state.project.clone();
            let mut edit = e.edit_project("Grain").unwrap();
            edit.update(|project| project.output.grain.amount = 0.12);
            e.commit_project(edit).unwrap();
            before
        });
        begin(app, "bare-light.dng");
        wait(app);
        let (reference, region) = app.read(|e| {
            let state = e.document.flat_edit().unwrap();
            (
                state.view.ready.as_ref().unwrap().developed.clone(),
                state.region,
            )
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            let state = e.document.flat_edit().unwrap();
            assert!(Arc::ptr_eq(
                &reference,
                &state.view.ready.as_ref().unwrap().developed
            ));
            assert_eq!(state.region, region);
        });
        apply(app);
        wait(app);
        app.read(|e| {
            assert!(e.document.flat_edit().is_none());
            assert!(e.document.correction.prepared().is_some(), "{}", e.status);
            assert_eq!(e.document.project.output, before.output);
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Correction")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}
