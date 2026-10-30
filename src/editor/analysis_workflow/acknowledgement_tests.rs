use super::tests::{LIMIT, fixture};
use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    source as source_loading,
};
use std::{cell::Cell, rc::Rc, sync::mpsc};

// Substitute only the numerical work. Initiation, decision ownership and completion
// still run through the feature continuation.
fn start_cpu(
    app: &TestApplication<'_, Editor>,
    fitting: bool,
    entered: mpsc::Sender<()>,
    resume: mpsc::Receiver<()>,
    returned: mpsc::Sender<()>,
    continued: Rc<Cell<bool>>,
) {
    fn work<T: 'static>(
        services: Services,
        entered: mpsc::Sender<()>,
        resume: mpsc::Receiver<()>,
        returned: mpsc::Sender<()>,
        continued: Rc<Cell<bool>>,
    ) -> testing::Work<T> {
        Box::new(move |_, stopped| {
            Box::pin(async move {
                let result = services
                    .cpu(&stopped, move || {
                        entered.send(()).unwrap();
                        resume.recv_timeout(LIMIT).unwrap();
                        returned.send(()).unwrap();
                        Ok(())
                    })
                    .await;
                continued.set(true);
                result.map(|_| None).map_err(|error| error.to_string())
            })
        })
    }
    app.enter(|editor, access| {
        let request = editor.analysis_request().unwrap();
        let services = editor.analysis_services().unwrap();
        if fitting {
            editor.document.analysis_fit_work =
                Some(work(services, entered, resume, returned, continued));
            editor.start_analysis(request, access);
        } else {
            editor.document.analysis_preparation_work =
                Some(work(services, entered, resume, returned, continued));
            editor.prepare_analysis_review(request, access);
        }
    });
}

#[test]
fn user_cancel_waits_for_running_cpu_acknowledgement_in_preparation_and_fitting() {
    for fitting in [false, true] {
        let scheduler = source_loading::SourceLoader::new();
        let editor = fixture(scheduler.clone());
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let (returned, returns) = mpsc::channel();
        let continued = Rc::new(Cell::new(false));
        with_application(editor, |app| {
            start_cpu(app, fitting, entered, resume, returned, continued.clone());
            app.pump();
            starts.recv_timeout(LIMIT).unwrap();
            app.read(|e| assert!(e.tasks.view().unwrap().cancel.is_some()));
            app.update(Editor::cancel_analysis);
            app.pump();
            app.read(|e| {
                assert!(e.busy && !e.file_available());
                let activity = e.tasks.view().unwrap();
                assert!(activity.cancel.is_none() && activity.measured.is_none());
                assert!(e.analysis_preparing() || e.analysis_fitting());
            });
            assert!(!continued.get());
            assert!(matches!(returns.try_recv(), Err(mpsc::TryRecvError::Empty)));
            release.send(()).unwrap();
            app.run_until(LIMIT, |e| !e.busy);
            app.read(|e| {
                assert!(e.tasks.view().is_none());
                assert!(!e.analysis_preparing() && !e.analysis_fitting());
                assert!(e.status.contains("cancelled"), "{}", e.status);
            });
            returns.recv_timeout(LIMIT).unwrap();
            assert!(continued.get());
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn application_retirement_drops_cpu_continuation_while_the_worker_finishes() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = fixture(scheduler.clone());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (returned, returns) = mpsc::channel();
    let continued = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        start_cpu(app, false, entered, resume, returned, continued.clone());
        app.pump();
        starts.recv_timeout(LIMIT).unwrap();
        let stopped = app.read(|e| e.document.analysis_operation().unwrap().cancelled.clone());
        app.close_execution();
        assert!(stopped.is_cancelled() && !continued.get());
        release.send(()).unwrap();
        returns.recv_timeout(LIMIT).unwrap();
    });
    futures_lite::future::block_on(crate::background::run(|_| {})).unwrap();
    assert!(!continued.get());
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancellation_before_first_poll_acknowledges_without_starting_a_cpu_stage() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = fixture(scheduler.clone());
    let (entered, starts) = mpsc::channel();
    let (_release, resume) = mpsc::channel();
    let (returned, _returns) = mpsc::channel();
    with_application(editor, |app| {
        start_cpu(
            app,
            false,
            entered,
            resume,
            returned,
            Rc::new(Cell::new(false)),
        );
        app.update(Editor::cancel_analysis);
        app.run_until(LIMIT, |e| !e.busy);
        assert!(matches!(
            starts.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        app.read(|e| assert!(!e.analysis_preparing() && e.tasks.view().is_none()));
    });
    scheduler.close_and_wait().unwrap();
}
