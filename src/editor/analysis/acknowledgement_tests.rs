use super::workflow_tests::{LIMIT, fixture};
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
        entered: mpsc::Sender<()>,
        resume: mpsc::Receiver<()>,
        returned: mpsc::Sender<()>,
        continued: Rc<Cell<bool>>,
    ) -> testing::Work<T> {
        Box::new(move |_, stopped| {
            Box::pin(async move {
                let result = background::compute(&stopped, move || {
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
        if fitting {
            editor.document.analysis_fit_work = Some(work(entered, resume, returned, continued));
            editor.start_analysis(request, access);
        } else {
            editor.document.analysis_preparation_work =
                Some(work(entered, resume, returned, continued));
            editor.prepare_analysis_review(request, access);
        }
    });
}

#[test]
fn cancel_during_actual_camera_capture_keeps_feedback_until_the_worker_acknowledges() {
    struct ObservedCapture {
        inner: Arc<dyn crate::source::SourceProvider>,
        observer: Arc<dyn Fn() + Send + Sync>,
    }
    impl crate::source::SourceProvider for ObservedCapture {
        fn load_source(
            &self,
            key: crate::source::SourceKey,
            priority: crate::source::Priority,
        ) -> crate::source::SourceFuture<'_> {
            self.inner.load_source(key, priority)
        }
        fn camera_capture_observer(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
            Some(self.observer.clone())
        }
    }
    let loader = source_loading::SourceLoader::new();
    let editor = fixture(loader.clone());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (returned, returns) = mpsc::channel();
    let resume = Mutex::new(resume);
    let provider = Arc::new(ObservedCapture {
        inner: loader.provider(None),
        observer: Arc::new(move || {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
            returned.send(()).unwrap();
        }),
    });
    let continued = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        let completed = continued.clone();
        app.enter(|editor, cx| {
            let request = editor.analysis_request().unwrap();
            editor.document.analysis_fit_work = Some(Box::new(move |request, stopped| {
                Box::pin(async move {
                    let id = request.document.project.order[0];
                    let physical = request
                        .document
                        .source_input(id, &Default::default())
                        .unwrap();
                    let captured = SamplingInputs::default()
                        .acquire(id, &physical, provider.as_ref(), &stopped)
                        .await
                        .map_err(|error| error.to_string())?;
                    completed.set(true);
                    assert!(captured.is_none());
                    Ok(None)
                })
            }));
            editor.start_analysis(request, cx);
        });
        let deadline = std::time::Instant::now() + LIMIT;
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "camera capture did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.update(Editor::cancel_analysis);
        app.pump();
        app.read(|editor| {
            assert!(editor.analysis_fitting());
            assert!(editor.interaction_blocked() && !editor.file_available());
            assert!(editor.activities.view().unwrap().cancel.is_none());
        });
        assert!(!continued.get());
        assert!(matches!(returns.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).unwrap();
        app.run_until(LIMIT, |editor| !editor.interaction_blocked());
        returns.recv_timeout(LIMIT).unwrap();
        assert!(continued.get());
        app.read(|editor| {
            assert!(editor.activities.view().is_none());
            assert!(editor.status.contains("cancelled"), "{}", editor.status);
        });
    });
    loader.close_and_wait().unwrap();
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
            app.read(|e| assert!(e.activities.view().unwrap().cancel.is_some()));
            app.update(Editor::cancel_analysis);
            app.pump();
            app.read(|e| {
                assert!(e.interaction_blocked() && !e.file_available());
                let activity = e.activities.view().unwrap();
                assert!(activity.cancel.is_none() && activity.measured.is_none());
                assert!(e.analysis_preparing() || e.analysis_fitting());
            });
            assert!(!continued.get());
            assert!(matches!(returns.try_recv(), Err(mpsc::TryRecvError::Empty)));
            release.send(()).unwrap();
            app.run_until(LIMIT, |e| !e.interaction_blocked());
            app.read(|e| {
                assert!(e.activities.view().is_none());
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
        app.run_until(LIMIT, |e| !e.interaction_blocked());
        assert!(matches!(
            starts.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        app.read(|e| assert!(!e.analysis_preparing() && e.activities.view().is_none()));
    });
    scheduler.close_and_wait().unwrap();
}
