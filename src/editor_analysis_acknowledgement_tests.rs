use super::tests::{LIMIT, fixture};
use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::development,
};
use std::{cell::Cell, rc::Rc, sync::mpsc};

// Gate the actual CPU service boundary and production result delivery, not the
// numerical estimator: cancellation must not replace its acknowledgement.
fn start_cpu(
    app: &TestApplication<'_, Editor>,
    fitting: bool,
    entered: mpsc::Sender<()>,
    resume: mpsc::Receiver<()>,
    returned: mpsc::Sender<()>,
    continued: Rc<Cell<bool>>,
) {
    app.enter(|editor, access| {
        // Attach a real gated CPU task directly to its owner: these cases isolate
        // service acknowledgement and result delivery, not command initiation.
        editor.sync_document();
        let revision = editor
            .document
            .analysis_generation
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let id = editor.request_id();
        let request = analysis::Request {
            loaded: editor.id,
            revision,
            document: editor.document.capture(),
            selected: editor.current_source.unwrap(),
            film: None,
            local_support: false,
            picture: None,
            areas: None,
        };
        let pending = Pending::new(id, request);
        editor.busy = true;
        editor.tasks.begin(if fitting {
            super::super::Task::Analyze(id)
        } else {
            super::super::Task::AnalysisAreas(id)
        });
        if fitting {
            editor.document.analysis_pending = Some(pending);
        } else {
            editor.document.analysis_preparing = Some(pending);
        }
        let pending = if fitting {
            editor.document.analysis_pending.as_ref().unwrap()
        } else {
            editor.document.analysis_preparing.as_ref().unwrap()
        };
        let id = pending.id;
        let stopped = pending.cancelled.clone();
        let retire = Retire(stopped.clone());
        let services = Services::from(editor.source_services.as_ref().unwrap());
        let task = access
            .spawn(move |cx| async move {
                let _retire = retire;
                let work = services.cpu(&stopped, move || {
                    entered.send(()).unwrap();
                    resume.recv_timeout(LIMIT).unwrap();
                    returned.send(()).unwrap();
                    Ok(())
                });
                let result = drive(work, Arc::default(), &cx, id).await;
                continued.set(true);
                cx.update(|editor| {
                    if fitting {
                        editor.analysis_result(id, result.map(|_| None), &cx);
                    } else {
                        editor.analysis_areas_result(id, result.map(|_| None), &cx);
                    }
                });
            })
            .unwrap();
        let pending = if fitting {
            editor.document.analysis_pending.as_mut().unwrap()
        } else {
            editor.document.analysis_preparing.as_mut().unwrap()
        };
        pending.task = Some(task);
    });
}

#[test]
fn user_cancel_waits_for_running_cpu_acknowledgement_in_preparation_and_fitting() {
    for fitting in [false, true] {
        let scheduler = development::Scheduler::new();
        let editor = fixture(scheduler.clone());
        let jobs = editor.workflow_jobs.clone();
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
                assert!(
                    e.document.analysis_preparing.is_some()
                        || e.document.analysis_pending.is_some()
                );
            });
            assert!(!continued.get() && !jobs.idle());
            assert!(matches!(returns.try_recv(), Err(mpsc::TryRecvError::Empty)));
            release.send(()).unwrap();
            app.run_until(LIMIT, |e| !e.busy);
            app.read(|e| {
                assert!(e.tasks.view().is_none());
                assert!(
                    e.document.analysis_preparing.is_none()
                        && e.document.analysis_pending.is_none()
                );
                assert!(e.status.contains("cancelled"), "{}", e.status);
            });
            returns.recv_timeout(LIMIT).unwrap();
            assert!(continued.get());
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn application_retirement_drops_cpu_continuation_without_releasing_running_admission() {
    let scheduler = development::Scheduler::new();
    let editor = fixture(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (returned, returns) = mpsc::channel();
    let continued = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        start_cpu(app, false, entered, resume, returned, continued.clone());
        app.pump();
        starts.recv_timeout(LIMIT).unwrap();
        let stopped = app.read(|e| {
            e.document
                .analysis_preparing
                .as_ref()
                .unwrap()
                .cancelled
                .clone()
        });
        app.close_execution();
        assert!(stopped.is_cancelled() && !continued.get() && !jobs.idle());
        release.send(()).unwrap();
        returns.recv_timeout(LIMIT).unwrap();
    });
    futures_lite::future::block_on(jobs.run(|| {})).unwrap();
    assert!(jobs.idle() && !continued.get());
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancellation_before_first_poll_acknowledges_without_starting_a_cpu_stage() {
    let scheduler = development::Scheduler::new();
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
        app.read(|e| assert!(e.document.analysis_preparing.is_none() && e.tasks.view().is_none()));
    });
    scheduler.close_and_wait().unwrap();
}
