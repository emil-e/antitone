use super::*;

#[test]
fn positive_view_requests_and_presents_rendering_while_save_is_pending() {
    let mut editor = crate::editor::tests::accepted_input(
        crate::editor::tests::authored_fixture(&["patches.dng"]),
        InputSettings::Digital,
    );
    editor.document.source = false;
    editor.photo_settings_mut().exposure = 0.75;
    let (entered, reached) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel::<()>(0);
    let resume = std::sync::Mutex::new(resume);
    editor.document.write_observer = Some(Arc::new(move |phase| {
        if phase == crate::project::WritePhase::TemporarySynced {
            entered.send(()).unwrap();
            let _ = resume.lock().unwrap().recv();
        }
    }));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(path.clone(), crate::project::Save::New, None, cx)
        });
        app.pump();
        reached.recv_timeout(Duration::from_secs(15)).unwrap();
        let acquisition = crate::background::testing::pause();
        let context = egui::Context::default();
        for _ in 0..2 {
            crate::editor::tests::app_frame(app, &context, vec![]);
        }
        let requested = app.read(|editor| {
            assert!(editor.document.write.pending() && editor.interaction_blocked());
            let working = &editor.document.visit.working_view;
            assert!(
                working.task.is_some(),
                "Save must not prevent a view request"
            );
            assert!(!working.support_ready());
            let requested = working.wanted.clone().unwrap();
            assert!(!requested.capture.is_inspection());
            assert_eq!(
                requested.capture.recipe().unwrap().grade.layers()[0]
                    .photo
                    .exposure,
                0.75
            );
            requested
        });
        drop(acquisition);
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        crate::editor::tests::app_frame(app, &context, vec![]);
        app.read(|editor| {
            assert!(editor.document.write.pending() && editor.interaction_blocked());
            let working = &editor.document.visit.working_view;
            let displayed = working
                .displayed
                .as_ref()
                .expect("present the completed positive");
            let painted = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert!(Arc::ptr_eq(displayed, painted));
            assert!(displayed.capture == requested.capture);
            assert_eq!(displayed.spec.frame(), requested.frame);
            assert!(editor.active_canvas().preview_rendered.is_none());
        });
        drop(release);
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending()
        });
        assert!(path.is_file());
    });
}
