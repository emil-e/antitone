use super::*;
use crate::raw::fixture;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn request(path: &std::path::Path, store: Arc<SourceStore>) -> Request {
    let (expected, _) = document::open_source(path, Some([1.; 4])).unwrap();
    let file = thumbnails::FileKey::new(path);
    let key = SourceKey::from_revision(
        path,
        file.clone(),
        &expected.development,
        &Development::for_source(&expected.development, [1.; 4]),
    );
    Request {
        path: path.to_owned(),
        wb: Some([1.; 4]),
        expected: Some(expected),
        basis: None,
        file,
        key,
        store: Some(store),
        parent: None,
        retain: false,
    }
}

fn until(scheduler: &Scheduler, condition: impl Fn(&State) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let shared = &scheduler.owner.0;
    let mut state = shared.state.lock().unwrap();
    while !condition(&state) {
        assert!(
            std::time::Instant::now() < deadline,
            "scheduler state deadline"
        );
        state = shared
            .changed
            .wait_timeout(state, Duration::from_millis(10))
            .unwrap()
            .0;
    }
}

#[test]
fn foreground_runs_before_the_next_background_native_phase_on_one_thread() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let store = Arc::new(SourceStore::new(0));
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let worker_trace = trace.clone();
    let scheduler = Scheduler::start(move |shared| {
        let mut phases = HashMap::new();
        run(shared, |task| {
            let phase = phases.entry(task.id).or_insert(0);
            *phase += 1;
            worker_trace
                .lock()
                .unwrap()
                .push((task.id, *phase, std::thread::current().id()));
            if task.id == 1 && *phase == 1 {
                entered.send(()).unwrap();
                resume.recv().unwrap();
            }
            Ok((*phase == 2).then(|| source.clone()))
        });
    });
    let bg = scheduler.clone();
    let bg_request = request(a.path(), store.clone());
    let background = std::thread::spawn(move || bg.load(bg_request, Priority::Eager, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let fg = scheduler.clone();
    let fg_request = request(b.path(), store);
    let foreground =
        std::thread::spawn(move || fg.load(fg_request, Priority::Foreground, || false));
    until(&scheduler, |state| state.tasks.len() == 2);
    release.send(()).unwrap();
    foreground.join().unwrap().unwrap();
    background.join().unwrap().unwrap();
    let trace = trace.lock().unwrap();
    assert_eq!(
        trace
            .iter()
            .map(|(id, phase, _)| (*id, *phase))
            .collect::<Vec<_>>(),
        [(1, 1), (2, 1), (2, 2), (1, 2)]
    );
    assert!(trace.iter().all(|(_, _, thread)| *thread == trace[0].2));
}

#[test]
fn foreground_joins_started_work_after_thumbnail_waiter_cancels_at_zero_budget() {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let expected_source = source.clone();
    let store = Arc::new(SourceStore::new(0));
    let job = request(file.path(), store.clone());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let scheduler = Scheduler::start(move |shared| {
        run(shared, |task| {
            let call = worker_calls.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                Ok(None)
            } else {
                assert_eq!(task.id, 1);
                Ok(Some(source.clone()))
            }
        })
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let bg_cancelled = cancelled.clone();
    let bg = scheduler.clone();
    let bg_request = job.clone();
    let background = std::thread::spawn(move || {
        bg.load(bg_request, Priority::Eager, || {
            bg_cancelled.load(Ordering::Relaxed)
        })
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    cancelled.store(true, Ordering::Relaxed);
    assert!(background.join().unwrap().is_err());
    let fg = scheduler.clone();
    let foreground = std::thread::spawn(move || fg.load(job, Priority::Foreground, || false));
    until(&scheduler, |state| {
        state.tasks.len() == 1 && state.tasks[0].priority() == Priority::Foreground
    });
    release.send(()).unwrap();
    let completed = foreground.join().unwrap().unwrap();
    assert!(Arc::ptr_eq(&completed, &expected_source));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(store.charges().retained, 0);
}

#[test]
fn closing_worker_wakes_waiters_without_starting_a_second_development() {
    let file = fixture::bayer_dng();
    let job = request(file.path(), Arc::new(SourceStore::new(0)));
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let scheduler = Scheduler::start(move |shared| {
        run(shared, |_| {
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Err("closed native phase".into())
        })
    });
    let client = scheduler.clone();
    let waiter = std::thread::spawn(move || client.load(job, Priority::Foreground, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    scheduler.close();
    assert!(waiter.join().unwrap().is_err());
    release.send(()).unwrap();
}

#[test]
fn actual_single_worker_development_preserves_frozen_pixels_and_native_charge() {
    let file = fixture::bayer_dng();
    let scheduler = Scheduler::new();
    let store = Arc::new(SourceStore::new(8 << 20));
    let mut job = request(file.path(), store.clone());
    job.retain = true;
    let key = job.key.clone().unwrap();
    let expected = Scan::open(file.path(), Some([1.; 4])).unwrap();
    let result = scheduler.load(job, Priority::Eager, || false).unwrap();
    assert!(Arc::ptr_eq(&store.get(&key).unwrap(), &result));
    assert!(store.charges().retained <= 8 << 20);
    let a = expected.source().camera_image().unwrap();
    let b = result.camera_image().unwrap();
    for y in 0..a.height {
        for x in 0..a.width {
            assert_eq!(
                a.pixel_samples(x, y).unwrap(),
                b.pixel_samples(x, y).unwrap()
            );
        }
    }
    assert_eq!(
        format!(
            "{:?}",
            result.region_sensor_diagnostics(expected.area).unwrap()
        ),
        format!(
            "{:?}",
            expected
                .source()
                .region_sensor_diagnostics(expected.area)
                .unwrap()
        )
    );
}

#[test]
fn cancelled_queued_demand_never_develops_or_publishes() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let store = Arc::new(SourceStore::new(0));
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let scheduler = Scheduler::start(move |shared| {
        run(shared, |task| {
            assert_eq!(task.id, 1);
            worker_calls.fetch_add(1, Ordering::Relaxed);
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Ok(Some(source.clone()))
        })
    });
    let main_request = request(a.path(), store.clone());
    let main_scheduler = scheduler.clone();
    let main = std::thread::spawn(move || {
        main_scheduler.load(main_request, Priority::Foreground, || false)
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued_cancelled = cancelled.clone();
    let queued_request = request(b.path(), store.clone());
    let queued_scheduler = scheduler.clone();
    let queued = std::thread::spawn(move || {
        queued_scheduler.load(queued_request, Priority::Eager, || {
            queued_cancelled.load(Ordering::Relaxed)
        })
    });
    until(&scheduler, |state| state.tasks.len() == 2);
    cancelled.store(true, Ordering::Relaxed);
    assert!(queued.join().unwrap().is_err());
    release.send(()).unwrap();
    main.join().unwrap().unwrap();
    until(&scheduler, |state| state.tasks.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.charges().retained, 0);
}

#[test]
fn closing_a_candidate_cancels_only_its_joined_waiter() {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let parent = Arc::new(SourceStore::new(0));
    let candidate = Arc::new(SourceStore::new(0));
    let parent_request = request(file.path(), parent.clone());
    let mut candidate_request = request(file.path(), candidate.clone());
    candidate_request.parent = Some(parent.clone());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let scheduler = Scheduler::start(move |shared| {
        run(shared, |task| {
            assert_eq!(task.id, 1);
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Ok(Some(source.clone()))
        })
    });
    let main_scheduler = scheduler.clone();
    let main = std::thread::spawn(move || {
        main_scheduler.load(parent_request, Priority::Visible, || false)
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let candidate_scheduler = scheduler.clone();
    let abandoned = std::thread::spawn(move || {
        candidate_scheduler.load(candidate_request, Priority::Foreground, || false)
    });
    until(&scheduler, |state| {
        state.tasks.len() == 1 && state.tasks[0].demands.lock().unwrap().len() == 2
    });
    candidate.close();
    assert!(abandoned.join().unwrap().is_err());
    assert!(!parent.is_closed());
    release.send(()).unwrap();
    main.join().unwrap().unwrap();
}

#[test]
fn native_failure_fans_out_to_joined_waiters_and_allows_a_new_attempt() {
    let file = fixture::bayer_dng();
    let job = request(file.path(), Arc::new(SourceStore::new(0)));
    let retry = job.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let scheduler = Scheduler::start(move |shared| {
        run(shared, |_| {
            if worker_calls.fetch_add(1, Ordering::Relaxed) == 0 {
                entered.send(()).unwrap();
                resume.recv().unwrap();
            }
            Err("decoder failure".into())
        })
    });
    let bg_request = job.clone();
    let bg_scheduler = scheduler.clone();
    let bg = std::thread::spawn(move || bg_scheduler.load(bg_request, Priority::Eager, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let fg_scheduler = scheduler.clone();
    let fg = std::thread::spawn(move || fg_scheduler.load(job, Priority::Foreground, || false));
    until(&scheduler, |state| {
        state.tasks.len() == 1 && state.tasks[0].demands.lock().unwrap().len() == 2
    });
    release.send(()).unwrap();
    assert!(
        bg.join()
            .unwrap()
            .err()
            .unwrap()
            .to_string()
            .contains("decoder failure")
    );
    assert!(
        fg.join()
            .unwrap()
            .err()
            .unwrap()
            .to_string()
            .contains("decoder failure")
    );
    assert!(
        scheduler
            .load(retry, Priority::Foreground, || false)
            .is_err()
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn missing_keys_different_revisions_and_unrelated_scopes_do_not_join() {
    let file = fixture::bayer_dng();
    let store = Arc::new(SourceStore::new(0));
    let original = request(file.path(), store.clone());
    let mut unversioned = original.clone();
    unversioned.key = None;
    assert!(!unversioned.joins(&original));
    let other_document = request(file.path(), Arc::new(SourceStore::new(0)));
    assert!(!other_document.joins(&original));
    use std::io::Write;
    let mut modified = std::fs::OpenOptions::new()
        .append(true)
        .open(file.path())
        .unwrap();
    modified.write_all(&[0]).unwrap();
    let changed = request(file.path(), store);
    assert!(!changed.joins(&original));
    assert!(!original.key.unwrap().current());
}

#[test]
fn parent_joins_candidate_first_work_and_survives_candidate_retirement() {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let parent = Arc::new(SourceStore::new(8 << 20));
    let candidate = Arc::new(SourceStore::new(0));
    let mut candidate_request = request(file.path(), candidate.clone());
    candidate_request.parent = Some(parent.clone());
    candidate_request.retain = true;
    let mut parent_request = request(file.path(), parent.clone());
    parent_request.retain = true;
    let key = parent_request.key.clone().unwrap();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let scheduler = Scheduler::start(move |shared| {
        run(shared, |task| {
            assert_eq!(task.id, 1);
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Ok(Some(source.clone()))
        })
    });
    let child_scheduler = scheduler.clone();
    let child = std::thread::spawn(move || {
        child_scheduler.load(candidate_request, Priority::Foreground, || false)
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let parent_scheduler = scheduler.clone();
    let accepted = std::thread::spawn(move || {
        parent_scheduler.load(parent_request, Priority::Visible, || false)
    });
    until(&scheduler, |state| {
        state.tasks.len() == 1 && state.tasks[0].demands.lock().unwrap().len() == 2
    });
    candidate.close();
    assert!(child.join().unwrap().is_err());
    release.send(()).unwrap();
    let handed_off = accepted.join().unwrap().unwrap();
    assert!(Arc::ptr_eq(&parent.get(&key).unwrap(), &handed_off));
    assert!(candidate.is_closed());
    assert_eq!(candidate.charges().retained, 0);
}

#[test]
fn diagnostic_stop_before_unpack_rejects_without_development_or_publication() {
    let file = fixture::bayer_dng();
    let store = Arc::new(SourceStore::new(8 << 20));
    let mut job = request(file.path(), store.clone());
    job.retain = true;
    let key = job.key.clone().unwrap();
    let phases = Arc::new(Mutex::new(Vec::new()));
    let observed = phases.clone();
    let scheduler = Scheduler::new_observed(move |_, phase, _| {
        observed.lock().unwrap().push(format!("{phase:?}"));
        if matches!(phase, NativePhase::UnpackBegin) {
            Err("diagnostic stop".into())
        } else {
            Ok(())
        }
    });
    assert!(
        scheduler
            .load(job, Priority::Foreground, || false)
            .err()
            .unwrap()
            .to_string()
            .contains("diagnostic stop")
    );
    assert_eq!(*phases.lock().unwrap(), ["Open", "UnpackBegin"]);
    assert!(store.get(&key).is_none());
    assert_eq!(store.charges().retained, 0);
}
