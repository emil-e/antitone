use super::*;
use crate::{
    application::test_support::with_application,
    editor::{Preparation, Prepared, development, document::Document},
};
use std::{
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::Duration,
};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")
}

fn editor(scheduler: development::Scheduler) -> Editor {
    let mut editor = super::super::tests::fixture();
    let mut document = Document::default();
    document.import(&[path()]).unwrap();
    editor.document.install(document);
    editor
        .thumbnails
        .set_limits(super::super::cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    editor.source_services = Some(source::Services {
        developments: scheduler,
        jobs: editor.workflow_jobs.clone(),
        renderers: Arc::default(),
        device,
        queue,
    });
    editor
}

fn paused() -> (development::Scheduler, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    (scheduler, starts, release)
}

fn start(app: &crate::application::test_support::TestApplication<'_, Editor>) {
    app.enter(|editor, access| {
        editor.load(path());
        editor.start_source_workflow(access);
    });
}

#[test]
fn retiring_source_ownership_does_not_clear_an_unrelated_document_activity() {
    let mut editor = super::super::tests::fixture();
    editor.load("not-opened.dng".into());
    editor.start_document(Preparation::New);
    let operation = editor.document_pending;
    let activity = editor.tasks.view().unwrap().title;
    let view = editor.request_id();
    editor.replace_photo_scope(view);
    assert!(editor.busy);
    assert_eq!(editor.document_pending, operation);
    assert_eq!(editor.tasks.view().unwrap().title, activity);
}

#[test]
fn requested_source_becomes_ready_without_painting_and_preserves_authored_pixels() {
    let scheduler = development::Scheduler::new();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            let ready = e.document.visit.ready.as_ref().unwrap();
            assert_eq!(ready.path, path());
            assert!(!e.document.visit.source.pending());
            assert!(e.tasks.view().is_none());
            assert_eq!(e.document.cache().charges().retained, 0);
            let mut scan = crate::scan::Scan::from_source(ready.developed.clone());
            let value = scan
                .reference(crate::raw::Rect {
                    x: 24,
                    y: 24,
                    width: 16,
                    height: 16,
                })
                .unwrap();
            assert!(
                (value - glam::DVec3::splat(3000. / 4031.))
                    .abs()
                    .max_element()
                    <= 2. / 65535.
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retired_visit_cannot_publish_and_running_native_work_keeps_foreground_admission() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        start(app);
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        app.update(|e| {
            let view = e.request_id();
            e.id = view;
            e.replace_photo_scope(view);
            e.status = "new visit".into();
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 0);
        let (entered, observed) = mpsc::channel();
        let access = app.access();
        let probe = access
            .spawn(async move {
                jobs.run(move || entered.send(()).unwrap()).await.unwrap();
            })
            .unwrap();
        app.pump();
        assert!(matches!(
            observed.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        drop(probe);
        app.read(|e| {
            assert!(e.document.visit.ready.is_none());
            assert_eq!(e.status, "new visit");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_an_unstarted_native_demand_releases_admission_without_waiting_for_an_unrelated_call()
{
    let (scheduler, starts, release) = paused();
    let background = scheduler
        .submit(
            development::Request {
                path: path(),
                wb: None,
                expected: None,
                basis: None,
                file: super::super::thumbnails::FileKey::new(&path()),
                key: None,
                store: None,
                parent: None,
                retain: false,
            },
            development::Priority::Eager,
        )
        .unwrap();
    starts.recv_timeout(Duration::from_secs(10)).unwrap();
    let editor = editor(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        start(app);
        app.run_until(Duration::from_secs(10), |_| scheduler.waiter_count() == 2);
        app.update(|e| {
            let view = e.request_id();
            e.replace_photo_scope(view);
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 1);
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(async move {
                jobs.run(move || entered.send(()).unwrap()).await.unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        drop(probe);
    });
    release.send(()).unwrap();
    drop(background);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn closed_worker_retains_parked_native_backing_until_another_call_returns_and_cleanup_finishes() {
    let (parked, at_park) = mpsc::channel();
    let (continue_a, resume_a) = mpsc::channel();
    let resume_a = Mutex::new(resume_a);
    let (entered_b, at_b) = mpsc::channel();
    let (continue_b, resume_b) = mpsc::channel();
    let resume_b = Mutex::new(resume_b);
    let scheduler = development::Scheduler::new_observed(move |file, phase, _| {
        if file.ends_with("patches.dng") && matches!(phase, development::NativePhase::UnpackEnd) {
            parked.send(()).unwrap();
            resume_a
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        if file.ends_with("roll-0.dng") && matches!(phase, development::NativePhase::UnpackBegin) {
            entered_b.send(()).unwrap();
            resume_b
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    let editor = editor(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        start(app);
        app.pump();
        at_park.recv_timeout(Duration::from_secs(10)).unwrap();
        app.update(|e| {
            let view = e.request_id();
            e.replace_photo_scope(view);
        });
        app.pump();
        let other = path().with_file_name("roll-0.dng");
        let ticket = scheduler
            .submit(
                development::Request {
                    file: super::super::thumbnails::FileKey::new(&other),
                    path: other,
                    wb: None,
                    expected: None,
                    basis: None,
                    key: None,
                    store: None,
                    parent: None,
                    retain: false,
                },
                development::Priority::Visible,
            )
            .unwrap();
        continue_a.send(()).unwrap();
        at_b.recv_timeout(Duration::from_secs(10)).unwrap();
        scheduler.close();
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(async move {
                jobs.run(move || entered.send(()).unwrap()).await.unwrap();
            })
            .unwrap();
        app.pump();
        assert!(
            matches!(
                observed.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "parked unpacked source still owns mandatory native backing"
        );
        continue_b.send(()).unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        drop((probe, ticket));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn worker_closure_finishes_the_waiter_but_holds_admission_until_the_native_call_returns() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        start(app);
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        scheduler.close();
        app.run_until(Duration::from_secs(10), |e| !e.busy);
        assert!(app.read(|e| e.document.visit.ready.is_none()));
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(async move {
                jobs.run(move || entered.send(()).unwrap()).await.unwrap();
            })
            .unwrap();
        app.pump();
        assert!(matches!(
            observed.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        drop(probe);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn failed_candidate_keeps_pending_source_and_defers_its_completion_without_losing_it() {
    for complete_first in [false, true] {
        let (scheduler, starts, release) = paused();
        let editor = editor(scheduler.clone());
        with_application(editor, |app| {
            start(app);
            app.pump();
            starts.recv_timeout(Duration::from_secs(10)).unwrap();
            let (view, accepted) =
                app.read(|e| (e.document.visit.view, e.document.project.clone()));
            app.update(|e| e.start_document(Preparation::Open("unavailable.antitone".into())));
            let operation = app.read(|e| e.document_pending.unwrap());
            assert!(app.read(|e| e.candidate_operation.request.is_some()));
            if complete_first {
                release.send(()).unwrap();
                app.run_until(Duration::from_secs(15), |e| {
                    e.document.visit.source.completion.is_some()
                });
                app.read(|e| {
                    assert!(e.busy && e.document.visit.ready.is_none());
                    assert_eq!(e.document_pending, Some(operation));
                    assert_eq!(e.document.project, accepted);
                });
            }
            app.update(|editor| {
                editor.document_prepared(operation, Err("candidate rejected".into()))
            });
            if !complete_first {
                assert!(app.read(|e| e.busy && e.document.visit.source.pending()));
                release.send(()).unwrap();
                app.run_until(Duration::from_secs(15), |e| !e.busy);
            }
            app.read(|e| {
                assert_eq!(e.document.visit.view, view);
                assert!(e.document.visit.ready.is_some());
                assert_eq!(e.document.project, accepted);
                assert!(e.status.contains("candidate rejected"));
                assert!(e.tasks.view().is_none());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn save_preserves_pending_visit_and_later_edits_in_both_completion_orders() {
    for complete_first in [false, true] {
        let (scheduler, starts, release) = paused();
        let editor = editor(scheduler.clone());
        let directory = tempfile::tempdir().unwrap();
        with_application(editor, |app| {
            start(app);
            app.pump();
            starts.recv_timeout(Duration::from_secs(10)).unwrap();
            let view = app.read(|e| e.document.visit.view);
            app.update(|e| {
                e.queue_save(
                    directory.path().join("written.antitone"),
                    crate::project::Save::New,
                    None,
                )
            });
            let operation = app.read(|e| e.document_pending.unwrap());
            let super::super::document_services::Write {
                document: snapshot,
                path,
                policy,
            } = app.update(|e| e.document.write.request.take().unwrap());
            let saved = snapshot.save(&path, policy).unwrap();
            app.update(|e| {
                e.roll.exposure = 0.5;
                e.sync_document();
            });
            if complete_first {
                release.send(()).unwrap();
                app.run_until(Duration::from_secs(15), |e| {
                    e.document.visit.source.completion.is_some()
                });
                assert!(app.read(|e| e.document.visit.ready.is_none() && e.busy));
            }
            app.update(|e| e.save_result(operation, Ok(Box::new(saved))));
            if !complete_first {
                assert!(app.read(|e| e.busy && e.document.visit.source.pending()));
                release.send(()).unwrap();
                app.run_until(Duration::from_secs(15), |e| !e.busy);
            }
            app.read(|e| {
                assert_eq!(e.document.visit.view, view);
                assert!(e.document.visit.ready.is_some() && e.tasks.view().is_none());
                assert!(e.document.dirty());
                assert_eq!(e.document.project.roll.exposure, 0.5);
                assert_eq!(e.document.path.as_deref(), Some(path.as_path()));
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn same_source_candidate_supplies_readiness_without_replacing_the_pending_visit() {
    let scheduler = development::Scheduler::new();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        let view = app.read(|e| e.document.visit.view);
        app.update(|e| {
            e.start_document(Preparation::Correction(
                e.document.capture(),
                e.current_source,
            ))
        });
        let operation = app.read(|e| e.document_pending.unwrap());
        assert!(app.read(|e| e.candidate_operation.request.is_some()));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.source.completion.is_some()
        });
        // Reuse the immutable prepared product; document preparation is a separate
        // boundary. This case exercises pending-visit promotion, not its algorithm.
        let prepared = app.read(|e| {
            let mut document = e.document.capture();
            document.fork_cache();
            Prepared {
                document,
                ready: Some(
                    (**e.document
                        .visit
                        .source
                        .completion
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .unwrap())
                    .clone(),
                ),
                selected: e.current_source,
                reuse_ready: false,
                correction: None,
                preserve_view: true,
            }
        });
        app.update(|editor| editor.document_prepared(operation, Ok(Box::new(prepared))));
        app.read(|e| {
            assert_eq!(e.document.visit.view, view);
            assert_eq!(e.id, operation);
            assert!(!e.document.visit.source.pending());
            assert!(e.document.visit.ready.is_some() && !e.busy);
            assert!(!e.document.cache().is_closed());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn replacing_document_retires_pending_source_but_not_its_running_native_call() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        start(app);
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        let store = app.read(|e| e.document.cache().clone());
        app.update(|e| e.start_document(Preparation::New));
        let operation = app.read(|e| e.document_pending.unwrap());
        assert!(app.read(|e| e.candidate_operation.request.is_some()));
        app.update(|editor| {
            editor.document_prepared(
                operation,
                Ok(Box::new(Prepared {
                    document: Document::default(),
                    ready: None,
                    selected: None,
                    reuse_ready: false,
                    correction: None,
                    preserve_view: false,
                })),
            )
        });
        app.pump();
        assert!(store.is_closed());
        assert_eq!(scheduler.waiter_count(), 0);
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(async move {
                jobs.run(move || entered.send(()).unwrap()).await.unwrap();
            })
            .unwrap();
        app.pump();
        assert!(matches!(
            observed.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        drop(probe);
        app.read(|e| {
            assert!(e.document.project.sources.is_empty());
            assert!(e.document.visit.ready.is_none());
            assert!(!e.busy && e.tasks.view().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
