use super::*;
use crate::application::test_support::with_application;
use std::{cell::Cell, rc::Rc, sync::mpsc, time::Duration};

#[test]
fn a_held_reservation_blocks_async_job_admission() {
    let jobs = BlockingJobs::new(1);
    let reservation = jobs.hold();
    assert!(jobs.0.try_acquire_arc().is_none());
    with_application(false, |app| {
        let attempted = Rc::new(Cell::new(false));
        let attempt = attempted.clone();
        let target = app.access();
        let pending = target
            .clone()
            .spawn(async move {
                attempt.set(true);
                let result = jobs.run(|| 42).await.unwrap();
                target.update(|done| *done = result == 42);
            })
            .unwrap();
        app.pump();
        assert!(attempted.get(), "the workflow must attempt admission");
        assert!(!app.read(|done| *done));
        drop(reservation);
        app.run_until(Duration::from_secs(5), |done| *done);
        drop(pending);
    });
}

#[test]
fn a_cancelled_waiter_does_not_release_running_work_to_a_blocking_waiter() {
    let jobs = BlockingJobs::new(1);
    let first = jobs.clone();
    let (started, starts) = mpsc::channel();
    let (release, released) = mpsc::channel();
    with_application((false, false), |app| {
        let target = app.access();
        let task = target
            .clone()
            .spawn(async move {
                first
                    .run(move || {
                        started.send(()).unwrap();
                        released.recv_timeout(Duration::from_secs(5)).unwrap();
                    })
                    .await
                    .unwrap();
                target.update(|state| state.0 = true);
            })
            .unwrap();
        app.pump();
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(task);
        app.pump();
        assert!(jobs.0.try_acquire_arc().is_none());
        let (entered, enters) = futures_channel::oneshot::channel();
        let blocking_waiter = std::thread::spawn(move || {
            let _admission = jobs.hold();
            let _ = entered.send(());
        });
        let target = app.access();
        let waiter = target
            .clone()
            .spawn(async move {
                enters.await.unwrap();
                target.update(|state| state.1 = true);
            })
            .unwrap();
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(5), |state| state.1);
        assert!(!app.read(|state| state.0), "departed continuation resumed");
        blocking_waiter.join().unwrap();
        drop(waiter);
    });
}

#[test]
fn cancelling_a_running_job_keeps_its_admission_until_the_worker_returns() {
    let jobs = BlockingJobs::new(1);
    let readiness = jobs.clone();
    assert!(readiness.idle());
    let first = jobs.clone();
    let (started, starts) = mpsc::channel();
    let (release, released) = mpsc::channel();
    with_application(false, |app| {
        let task = app
            .access()
            .spawn(async move {
                first
                    .run(move || {
                        started.send(()).unwrap();
                        released.recv_timeout(Duration::from_secs(5)).unwrap();
                    })
                    .await
            })
            .unwrap();
        app.pump();
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(task);
        app.pump();
        assert!(
            !readiness.idle(),
            "background admission must not mistake logical retirement for physical idleness"
        );
        assert!(
            jobs.0.try_acquire_arc().is_none(),
            "running work retains its permit after its waiter is destroyed"
        );
        let attempted = Rc::new(Cell::new(false));
        let attempt = attempted.clone();
        let target = app.access();
        let next = target
            .clone()
            .spawn(async move {
                attempt.set(true);
                jobs.run(|| ()).await.unwrap();
                target.update(|done| *done = true);
            })
            .unwrap();
        app.pump();
        assert!(attempted.get(), "the queued job must attempt admission");
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(5), |done| *done);
        assert!(readiness.idle());
        drop(next);
    });
}

#[test]
fn cancellable_admission_retires_a_queued_job_without_waiting_for_unrelated_work() {
    let jobs = BlockingJobs::new(1);
    let hold = jobs.hold();
    let stopped = Arc::new(event_listener::Event::default());
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    with_application(false, |app| {
        let target = app.access();
        let queued = jobs.clone();
        let stop = stopped.clone();
        let entry = entered.clone();
        let task = target
            .clone()
            .spawn(async move {
                let listener = stop.listen();
                let result = queued
                    .run_admitted_cancellable(
                        async {
                            listener.await;
                        },
                        move |_| {
                            entry.store(true, std::sync::atomic::Ordering::Relaxed);
                        },
                    )
                    .await;
                target.update(|done| *done = result.is_none());
            })
            .unwrap();
        app.pump();
        assert!(!app.read(|done| *done));
        stopped.notify(usize::MAX);
        app.run_until(Duration::from_secs(5), |done| *done);
        assert!(!entered.load(std::sync::atomic::Ordering::Relaxed));
        assert!(!jobs.idle(), "the unrelated reservation is still held");
        drop(hold);
        assert!(jobs.idle());
        drop(task);
    });
}

#[test]
fn admission_cancellation_does_not_replace_an_admitted_jobs_acknowledgement() {
    let jobs = BlockingJobs::new(1);
    let stopped = Arc::new(event_listener::Event::default());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    with_application(None, |app| {
        let target = app.access();
        let worker = jobs.clone();
        let stop = stopped.clone();
        let task = target
            .clone()
            .spawn(async move {
                let listener = stop.listen();
                let result = worker
                    .run_admitted_cancellable(
                        async {
                            listener.await;
                        },
                        move |_| {
                            entered.send(()).unwrap();
                            resume.recv_timeout(Duration::from_secs(5)).unwrap();
                            42
                        },
                    )
                    .await
                    .unwrap()
                    .unwrap();
                target.update(|value| *value = Some(result));
            })
            .unwrap();
        app.pump();
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        stopped.notify(usize::MAX);
        app.pump();
        assert!(app.read(Option::is_none) && !jobs.idle());
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(5), Option::is_some);
        assert_eq!(app.read(|value| *value), Some(42));
        assert!(jobs.idle());
        drop(task);
    });
}

#[test]
fn a_worker_panic_releases_admission_and_does_not_move_computation_onto_the_ui() {
    with_application(None, |app| {
        let jobs = BlockingJobs::new(1);
        let ui = std::thread::current().id();
        let target = app.access();
        let task = target
            .clone()
            .spawn(async move {
                assert!(jobs.run(|| panic!("job failed")).await.is_err());
                let worker = jobs.run(|| std::thread::current().id()).await.unwrap();
                target.update(|state| *state = Some(worker));
            })
            .unwrap();
        app.run_until(Duration::from_secs(5), Option::is_some);
        assert_ne!(app.read(|state| state.unwrap()), ui);
        drop(task);
    });
}
