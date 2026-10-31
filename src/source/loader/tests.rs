use super::*;
use crate::project::Development;
use crate::raw::fixture;
use std::sync::atomic::{AtomicBool, AtomicUsize};

fn request(path: &std::path::Path) -> Request {
    let (expected, _) = source_metadata::open_source(path, Some([1.; 4])).unwrap();
    let file = crate::source::FileKey::new(path);
    let key = SourceKey::from_revision_with_development(
        path,
        file.clone(),
        &expected.development,
        &Development::for_source(&expected.development, [1.; 4]),
    );

    Request {
        path: path.to_owned(),
        wb: Some([1.; 4]),
        expected: Some(expected),
        key,
    }
}

fn until(scheduler: &SourceLoader, condition: impl Fn(&State) -> bool) {
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

struct Notify {
    id: u8,
    send: mpsc::Sender<u8>,
}

impl std::task::Wake for Notify {
    fn wake(self: Arc<Self>) {
        let _ = self.send.send(self.id);
    }
}

fn poll_ticket(
    ticket: &mut LoadFuture,
    id: u8,
    send: &mpsc::Sender<u8>,
) -> Poll<Result<Arc<DevelopedSource>, Error>> {
    let waker = Waker::from(Arc::new(Notify {
        id,
        send: send.clone(),
    }));
    Pin::new(ticket).poll(&mut Context::from_waker(&waker))
}

#[test]
fn async_joined_consumers_wake_independently_and_share_frozen_pixels() {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let expected = source.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let scheduler = SourceLoader::start(move |shared| {
        run(shared, 0, |_| {
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Ok(Some(source.clone()))
        })
    });
    let job = request(file.path());
    let mut abandoned = scheduler
        .submit_request(job.clone(), Priority::Eager)
        .unwrap();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut foreground = scheduler.submit_request(job, Priority::Foreground).unwrap();
    let (wake, wakes) = mpsc::channel();
    assert!(poll_ticket(&mut abandoned, 1, &wake).is_pending());
    assert!(poll_ticket(&mut foreground, 2, &wake).is_pending());
    drop(abandoned);
    assert_eq!(scheduler.waiter_count(), 1);
    release.send(()).unwrap();
    assert_eq!(wakes.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    let Poll::Ready(Ok(source)) = poll_ticket(&mut foreground, 2, &wake) else {
        panic!("completion was not published before wake")
    };
    assert!(Arc::ptr_eq(&source, &expected));
    assert!(wakes.try_recv().is_err());
}

#[test]
fn a_new_demand_during_native_retirement_starts_fresh_instead_of_receiving_retirement() {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let job = request(file.path());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (retiring, cleanup) = mpsc::channel();
    let (release_cleanup, resume_cleanup) = mpsc::channel();
    let scheduler = SourceLoader::start(move |shared| {
        let owner = shared.clone();
        let mut unpacked = false;
        run(shared, 0, move |task| {
            if owner.retire_if_unwanted(task) {
                retiring.send(()).unwrap();
                resume_cleanup.recv_timeout(Duration::from_secs(5)).unwrap();
                return Err("source demand retired".into());
            }
            if !unpacked {
                unpacked = true;
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(None)
            } else {
                Ok(Some(source.clone()))
            }
        });
    });
    let first = scheduler
        .submit_request(job.clone(), Priority::Foreground)
        .unwrap();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(first);
    release.send(()).unwrap();
    cleanup.recv_timeout(Duration::from_secs(5)).unwrap();
    let replacement = scheduler.submit_request(job, Priority::Foreground).unwrap();
    release_cleanup.send(()).unwrap();
    assert!(replacement.wait(|| false).is_ok());
    scheduler.close_and_wait().unwrap();
}

#[test]
fn async_native_waiters_observe_scheduler_closure_without_waiting_for_native_return() {
    {
        let file = fixture::bayer_dng();
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let scheduler = SourceLoader::start(move |shared| {
            run(shared, 0, |_| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                Err("retired native call".into())
            })
        });
        let mut ticket = scheduler
            .submit_request(request(file.path()), Priority::Foreground)
            .unwrap();
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        let (wake, wakes) = mpsc::channel();
        assert!(poll_ticket(&mut ticket, 1, &wake).is_pending());
        scheduler.close();
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            poll_ticket(&mut ticket, 1, &wake),
            Poll::Ready(Err(_))
        ));
        release.send(()).unwrap();
    }
}

#[test]
fn native_ticket_can_be_awaited_when_completion_precedes_its_first_poll() {
    let file = fixture::bayer_dng();
    let scheduler = SourceLoader::new();
    let ticket = scheduler
        .submit_request(request(file.path()), Priority::Foreground)
        .unwrap();
    let completed = ticket.wait(|| false).unwrap();
    let awaited = futures_lite::future::block_on(ticket).unwrap();
    assert!(Arc::ptr_eq(&completed, &awaited));
}

#[test]
fn foreground_runs_before_the_next_background_native_phase_on_one_thread() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let worker_trace = trace.clone();
    let scheduler = SourceLoader::start(move |shared| {
        let mut phases = HashMap::new();
        run(shared, 0, |task| {
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
    let bg_request = request(a.path());
    let background = std::thread::spawn(move || bg.load(bg_request, Priority::Eager, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let fg = scheduler.clone();
    let fg_request = request(b.path());
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
fn selecting_the_last_eager_source_promotes_its_existing_queued_load() {
    use crate::source::SourceCache;

    let first = fixture::bayer_dng();
    let middle = fixture::bayer_dng();
    let selected = fixture::bayer_dng();
    let sources: HashMap<_, _> = [&first, &middle, &selected]
        .into_iter()
        .map(|file| {
            let source = Scan::open(file.path(), Some([1.; 4]))
                .unwrap()
                .source()
                .clone();
            (file.path().to_owned(), source)
        })
        .collect();
    let first_path = first.path().to_owned();
    let blocked = first_path.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let recorded = trace.clone();
    let loader = SourceLoader::start(move |shared| {
        run(shared, 0, |task| {
            recorded.lock().unwrap().push(task.request.path.clone());
            if task.request.path == blocked {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok(Some(sources[&task.request.path].clone()))
        });
    });
    let owner = Arc::new(SourceCache::new(0));
    let cache = owner.provider(Arc::new(loader.clone()));
    let key = |path| request(path).key.unwrap();
    let mut first_load = Box::pin(cache.load_source(key(first.path()), Priority::Eager));
    assert!(
        futures_lite::future::block_on(futures_lite::future::poll_once(&mut first_load)).is_none()
    );
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut middle_load = Box::pin(cache.load_source(key(middle.path()), Priority::Eager));
    assert!(
        futures_lite::future::block_on(futures_lite::future::poll_once(&mut middle_load)).is_none()
    );
    let selected_key = key(selected.path());
    let mut eager = Box::pin(cache.load_source(selected_key.clone(), Priority::Eager));
    assert!(futures_lite::future::block_on(futures_lite::future::poll_once(&mut eager)).is_none());
    let mut foreground = Box::pin(cache.load_source(selected_key, Priority::Foreground));
    assert!(
        futures_lite::future::block_on(futures_lite::future::poll_once(&mut foreground)).is_none()
    );

    release.send(()).unwrap();
    futures_lite::future::block_on(foreground).unwrap();
    futures_lite::future::block_on(eager).unwrap();
    futures_lite::future::block_on(middle_load).unwrap();
    futures_lite::future::block_on(first_load).unwrap();
    loader.close_and_wait().unwrap();
    assert_eq!(
        *trace.lock().unwrap(),
        [
            first_path,
            selected.path().to_owned(),
            middle.path().to_owned()
        ],
        "selection must move ahead of queued eager work without a duplicate development"
    );
}

#[test]
fn foreground_joins_started_work_after_thumbnail_waiter_cancels() {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let expected_source = source.clone();
    let job = request(file.path());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let scheduler = SourceLoader::start(move |shared| {
        run(shared, 0, |task| {
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
}

#[test]
fn closing_worker_wakes_waiters_without_starting_a_second_development() {
    let file = fixture::bayer_dng();
    let job = request(file.path());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let scheduler = SourceLoader::start(move |shared| {
        run(shared, 0, |_| {
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
fn actual_native_development_preserves_frozen_pixels_and_native_charge() {
    let file = fixture::bayer_dng();
    let scheduler = SourceLoader::new();
    let job = request(file.path());
    let expected = Scan::open(file.path(), Some([1.; 4])).unwrap();
    let result = scheduler.load(job, Priority::Eager, || false).unwrap();
    assert_eq!(result.retained_bytes(), expected.source().retained_bytes());
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
}

/// Timing diagnostic for the two-owner schedule at production priorities: solo latency,
/// overlapped pair throughput and per-image contention, and foreground latency while
/// an eager background development runs. Wall times are printed, never asserted —
/// read the acceptance bars against a release run; private RAW samples are required.
#[test]
#[ignore = "explicit composed native-overlap diagnostic; needs private RAW samples"]
fn composed_native_overlap_throughput_and_foreground_latency() {
    use std::time::{Duration, Instant};
    let _session = std::env::var_os("ANTITONE_LOG_DIR").and_then(|_| crate::logging::init());
    fn same_development(a: &crate::scan::DevelopedSource, b: &crate::scan::DevelopedSource) {
        let a = a.camera_image().unwrap();
        let b = b.camera_image().unwrap();
        assert_eq!((a.width, a.height), (b.width, b.height));
        for y in 0..a.height {
            for x in 0..a.width {
                assert_eq!(
                    a.pixel_samples(x, y).unwrap(),
                    b.pixel_samples(x, y).unwrap()
                );
            }
        }
    }
    let root = std::env::var_os("ANTITONE_RAW_SAMPLES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples"));
    let paths: Vec<std::path::PathBuf> = ["DSC04378.ARW", "DSC04397.ARW"]
        .iter()
        .map(|name| root.join(name))
        .collect();
    for path in &paths {
        assert!(
            path.is_file(),
            "missing private RAW sample {}; set ANTITONE_RAW_SAMPLES",
            path.display()
        );
    }
    println!(
        "cpus: {}",
        std::thread::available_parallelism()
            .map(|c| c.get())
            .unwrap_or(0)
    );
    let native_calls = Arc::new(AtomicUsize::new(0));
    let observed = native_calls.clone();
    let scheduler = SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, NativePhase::DevelopBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    let fresh_schedule = || {
        until(&scheduler, |state| {
            state.tasks.is_empty() && state.steps == 0
        });
        // Oracle leases stay alive; each measured schedule must develop independently.
        scheduler.owner.0.state.lock().unwrap().live.clear();
        native_calls.load(Ordering::Relaxed)
    };
    let name = |i: usize| paths[i].file_name().unwrap().to_string_lossy().into_owned();
    let normal = || {
        matches!(
            crate::sys::memory_pressure(),
            crate::sys::MemoryPressure::Normal
        )
    };
    type Developed = std::sync::Arc<crate::scan::DevelopedSource>;
    type Timed = (Duration, Duration, Duration, Developed, Developed);
    let solo: Vec<Vec<(Duration, Developed)>> = paths
        .iter()
        .map(|path| {
            (0..3)
                .map(|rep| {
                    let before = fresh_schedule();
                    let job = request(path);
                    let start = Instant::now();
                    let developed = scheduler.load(job, Priority::Foreground, || false).unwrap();
                    let wall = start.elapsed();
                    assert_eq!(native_calls.load(Ordering::Relaxed) - before, 1);
                    println!(
                        "solo {} rep {} {:?} (pressure normal: {})",
                        path.file_name().unwrap().to_string_lossy(),
                        rep,
                        wall,
                        normal()
                    );
                    (wall, developed)
                })
                .collect()
        })
        .collect();
    let pairs: Vec<Timed> = (0..3)
        .map(|rep| {
            let before = fresh_schedule();
            let a_job = request(&paths[0]);
            let b_job = request(&paths[1]);
            let start = Instant::now();
            let ((a_wall, a), (b_wall, b)) = std::thread::scope(|scope| {
                let a_scheduler = scheduler.clone();
                let b_scheduler = scheduler.clone();
                let a = scope.spawn(move || {
                    let entered = Instant::now();
                    let developed = a_scheduler
                        .load(a_job, Priority::Visible, || false)
                        .unwrap();
                    (entered.elapsed(), developed)
                });
                let b = scope.spawn(move || {
                    let entered = Instant::now();
                    let developed = b_scheduler
                        .load(b_job, Priority::Foreground, || false)
                        .unwrap();
                    (entered.elapsed(), developed)
                });
                (a.join().unwrap(), b.join().unwrap())
            });
            let pair = start.elapsed();
            assert_eq!(native_calls.load(Ordering::Relaxed) - before, 2);
            println!(
                "yield pair rep {} {} {:?} | {} {:?} | pair {:?} (pressure normal: {})",
                rep,
                name(0),
                a_wall,
                name(1),
                b_wall,
                pair,
                normal()
            );
            (a_wall, b_wall, pair, a, b)
        })
        .collect();
    // Roll fill: two background demands with no foreground demand in flight. Both
    // owners admit them (no yield rule), so this measures the overlap throughput.
    let rolls: Vec<Timed> = (0..3)
        .map(|rep| {
            let before = fresh_schedule();
            let a_job = request(&paths[0]);
            let b_job = request(&paths[1]);
            let start = Instant::now();
            let ((a_wall, a), (b_wall, b)) = std::thread::scope(|scope| {
                let a_scheduler = scheduler.clone();
                let b_scheduler = scheduler.clone();
                let a = scope.spawn(move || {
                    let entered = Instant::now();
                    let developed = a_scheduler.load(a_job, Priority::Eager, || false).unwrap();
                    (entered.elapsed(), developed)
                });
                let b = scope.spawn(move || {
                    let entered = Instant::now();
                    let developed = b_scheduler.load(b_job, Priority::Eager, || false).unwrap();
                    (entered.elapsed(), developed)
                });
                (a.join().unwrap(), b.join().unwrap())
            });
            let roll = start.elapsed();
            assert_eq!(native_calls.load(Ordering::Relaxed) - before, 2);
            println!(
                "roll rep {} {} {:?} | {} {:?} | roll {:?} (pressure normal: {})",
                rep,
                name(0),
                a_wall,
                name(1),
                b_wall,
                roll,
                normal()
            );
            (a_wall, b_wall, roll, a, b)
        })
        .collect();
    let foreground_under_load: Vec<(Duration, Developed)> =
        (0..3)
            .map(|rep| {
                let before = fresh_schedule();
                let background_job = request(&paths[1]);
                let background_scheduler = scheduler.clone();
                let background = std::thread::spawn(move || {
                    let started = Instant::now();
                    let developed = background_scheduler
                        .load(background_job, Priority::Eager, || false)
                        .unwrap();
                    (started.elapsed(), developed)
                });
                // The background development is inside its unpack call when the
                // foreground demand arrives, as behind a live roll fill.
                std::thread::sleep(Duration::from_millis(200));
                let foreground_job = request(&paths[0]);
                let start = Instant::now();
                let developed = scheduler
                    .load(foreground_job, Priority::Foreground, || false)
                    .unwrap();
                let wall = start.elapsed();
                let (background_wall, _) = background.join().unwrap();
                assert_eq!(native_calls.load(Ordering::Relaxed) - before, 2);
                println!(
                    "foreground-under-load rep {} {} {:?} | background total {:?} (pressure normal: {})",
                    rep,
                    name(0),
                    wall,
                    background_wall,
                    normal()
                );
                (wall, developed)
            })
            .collect();
    // The overlapped and loaded schedules must produce the solo development.
    same_development(&rolls[0].3, &solo[0][0].1);
    same_development(&rolls[0].4, &solo[1][0].1);
    same_development(&pairs[0].3, &solo[0][0].1);
    same_development(&pairs[0].4, &solo[1][0].1);
    same_development(&foreground_under_load[0].1, &solo[0][0].1);
    let median = |walls: &[Duration]| {
        let mut sorted = walls.to_vec();
        sorted.sort();
        sorted[sorted.len() / 2]
    };
    let solo_median: Vec<Duration> = solo
        .iter()
        .map(|reps| median(&reps.iter().map(|(wall, _)| *wall).collect::<Vec<_>>()))
        .collect();
    let pair_per_image_median: Vec<Duration> = [0, 1]
        .iter()
        .map(|&i| {
            median(
                &pairs
                    .iter()
                    .map(|pair| if i == 0 { pair.0 } else { pair.1 })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let pair_median = median(&pairs.iter().map(|pair| pair.2).collect::<Vec<_>>());
    let roll_per_image_median: Vec<Duration> = [0, 1]
        .iter()
        .map(|&i| {
            median(
                &rolls
                    .iter()
                    .map(|roll| if i == 0 { roll.0 } else { roll.1 })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let roll_median = median(&rolls.iter().map(|roll| roll.2).collect::<Vec<_>>());
    let foreground_median = median(
        &foreground_under_load
            .iter()
            .map(|(wall, _)| *wall)
            .collect::<Vec<_>>(),
    );
    let contention = |overlapped: Duration, alone: Duration| {
        (overlapped.as_secs_f64() / alone.as_secs_f64() - 1.) * 100.
    };
    println!(
        "summary | solo medians {:?} {:?} | roll per-image {:?} {:?} roll {:?} | yield pair {:?} | foreground-under-load {:?}",
        solo_median[0],
        solo_median[1],
        roll_per_image_median[0],
        roll_per_image_median[1],
        roll_median,
        pair_median,
        foreground_median
    );
    println!(
        "summary | roll per-image contention {:.1}% / {:.1}% | roll throughput {:.2}x sequential | foreground-under-load vs solo {:.1}% | yield pair foreground {:?} vs solo {:?} (background {:?} yields)",
        contention(roll_per_image_median[0], solo_median[0]),
        contention(roll_per_image_median[1], solo_median[1]),
        (solo_median[0] + solo_median[1]).as_secs_f64() / roll_median.as_secs_f64(),
        contention(foreground_median, solo_median[0]),
        pair_per_image_median[1],
        solo_median[1],
        pair_per_image_median[0],
    );
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelled_queued_demand_never_runs_native_work() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let scheduler = SourceLoader::start(move |shared| {
        run(shared, 0, |task| {
            assert_eq!(task.id, 1);
            worker_calls.fetch_add(1, Ordering::Relaxed);
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Ok(Some(source.clone()))
        })
    });
    let main_request = request(a.path());
    let main_scheduler = scheduler.clone();
    let main = std::thread::spawn(move || {
        main_scheduler.load(main_request, Priority::Foreground, || false)
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued_cancelled = cancelled.clone();
    let queued_request = request(b.path());
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
}

#[test]
fn native_failure_fans_out_to_joined_waiters_and_allows_a_new_attempt() {
    let file = fixture::bayer_dng();
    let job = request(file.path());
    let retry = job.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = calls.clone();
    let scheduler = SourceLoader::start(move |shared| {
        run(shared, 0, |_| {
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
fn joining_uses_captured_source_identity_and_development() {
    let file = fixture::bayer_dng();
    let original = request(file.path());
    let mut unkeyed = original.clone();
    unkeyed.key = None;
    assert!(!unkeyed.joins(&original));
    assert!(request(file.path()).joins(&original));
    let key = original.key.as_ref().unwrap();
    let mut reopened_file = key.file().clone();
    reopened_file.revision.as_mut().unwrap().bytes += 1;
    let mut reopened = original.clone();
    reopened.key = Some(SourceKey::for_load(
        file.path(),
        reopened_file,
        key.development(),
    ));
    assert!(!reopened.joins(&original));
    let mut development = key.development().clone();
    development.decode.fixed_wb[0] = 2.;
    let mut different_wb = original.clone();
    different_wb.key = Some(SourceKey::for_load(
        file.path(),
        key.file().clone(),
        &development,
    ));
    assert!(!different_wb.joins(&original));
}

#[test]
fn diagnostic_stop_before_unpack_rejects_without_development() {
    let file = fixture::bayer_dng();
    let job = request(file.path());
    let phases = Arc::new(Mutex::new(Vec::new()));
    let observed = phases.clone();
    let scheduler = SourceLoader::new_observed(move |_, phase, _| {
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
}

fn synthetic_task(id: u64, path: &std::path::Path, priority: Priority) -> Arc<Task> {
    let task = Arc::new(Task::new(id, request(path)));
    task.demands.lock().unwrap().push(Consumer {
        id: 0,
        priority,
        waker: None,
    });
    task
}

#[test]
fn unowned_starts_yield_to_foreground_demand_and_started_work_stays_on_its_owner() {
    let file = fixture::bayer_dng();
    let foreground = synthetic_task(1, file.path(), Priority::Foreground);
    let background = synthetic_task(2, file.path(), Priority::Eager);
    let mut state = State {
        tasks: vec![foreground.clone(), background.clone()],
        live: Vec::new(),
        queue: vec![foreground.clone(), background.clone()],
        next: 2,
        steps: 0,
        sdk_steps: 0,
        closed: false,
    };
    assert!(!eligible(&state, &background, 0));
    assert!(!eligible(&state, &background, 1));
    assert!(eligible(&state, &foreground, 0));
    assert!(eligible(&state, &foreground, 1));
    state.tasks.retain(|task| task.id != 1);
    state.queue.retain(|task| task.id != 1);
    assert!(eligible(&state, &background, 0));
    assert!(eligible(&state, &background, 1));
    background.owner.store(0, Ordering::Relaxed);
    assert!(eligible(&state, &background, 0));
    assert!(!eligible(&state, &background, 1));
    foreground.owner.store(1, Ordering::Relaxed);
    assert!(!eligible(&state, &foreground, 0));
}

#[test]
fn sdk_steps_hold_native_execution_exclusive_so_they_keep_the_full_budget() {
    let file = fixture::bayer_dng();
    let sdk = synthetic_task(1, file.path(), Priority::Foreground);
    sdk.sdk.store(true, Ordering::Relaxed);
    let raw = synthetic_task(2, file.path(), Priority::Eager);
    let mut state = State {
        tasks: vec![sdk.clone(), raw.clone()],
        live: Vec::new(),
        queue: vec![sdk.clone(), raw.clone()],
        next: 2,
        steps: 0,
        sdk_steps: 0,
        closed: false,
    };
    // With nothing in flight both may start; an admitted SDK step therefore
    // records share 1 and keeps the undivided team the SDK route had alone.
    assert!(eligible(&state, &sdk, 0));
    assert!(eligible(&state, &sdk, 1));
    // Ordinary native work in flight blocks the serialized SDK transaction on
    // both owners, including the owner it would otherwise be pinned to.
    state.steps = 1;
    assert!(!eligible(&state, &sdk, 0));
    assert!(!eligible(&state, &sdk, 1));
    sdk.owner.store(1, Ordering::Relaxed);
    assert!(!eligible(&state, &sdk, 1));
    // A running SDK transaction blocks every other native step: the second
    // owner would otherwise block on the bridge's process-wide transaction
    // mutex while still halving the running call's thread team.
    sdk.owner.store(0, Ordering::Relaxed);
    state.tasks.retain(|task| task.id != 1);
    state.queue.retain(|task| task.id != 1);
    state.steps = 1;
    state.sdk_steps = 1;
    assert!(!eligible(&state, &raw, 0));
    assert!(!eligible(&state, &raw, 1));
    // Once it returns, ordinary work resumes.
    state.steps = 0;
    state.sdk_steps = 0;
    assert!(eligible(&state, &raw, 0));
    assert!(eligible(&state, &raw, 1));
}

#[test]
fn overlapping_owners_run_native_steps_on_separate_threads() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let barrier = Arc::new((Mutex::new(0usize), Condvar::new()));
    let threads = Arc::new(Mutex::new(Vec::new()));
    let recorded = threads.clone();
    let scheduler = SourceLoader::start_workers(2, move |_, _| {
        let barrier = barrier.clone();
        let threads = recorded.clone();
        let source = source.clone();
        Box::new(move |_| {
            threads.lock().unwrap().push(std::thread::current().id());
            let (arrived, cond) = &*barrier;
            let mut arrived = arrived.lock().unwrap();
            *arrived += 1;
            if *arrived < 2 {
                let (seen, _) = cond.wait_timeout(arrived, Duration::from_secs(5)).unwrap();
                assert!(*seen >= 2, "owners never overlapped native steps");
            }
            cond.notify_all();
            Ok(Some(source.clone()))
        })
    });
    let sa = scheduler.clone();
    let job_a = request(a.path());
    let sb = scheduler.clone();
    let job_b = request(b.path());
    let first = std::thread::spawn(move || sa.load(job_a, Priority::Eager, || false));
    let second = std::thread::spawn(move || sb.load(job_b, Priority::Eager, || false));
    let source_a = first.join().unwrap().unwrap();
    let source_b = second.join().unwrap().unwrap();
    assert!(Arc::ptr_eq(&source_a, &source_b));
    let threads = threads.lock().unwrap();
    assert_eq!(threads.len(), 2);
    assert_ne!(threads[0], threads[1], "both steps ran on one owner");
}

#[test]
fn foreground_starts_on_the_free_owner_while_a_background_step_runs() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let expected = source.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Arc::new(Mutex::new(resume));
    let scheduler = SourceLoader::start_workers(2, move |_, _| {
        let entered = entered.clone();
        let resume = resume.clone();
        let source = source.clone();
        Box::new(move |task| {
            if task.id == 1 {
                entered.send(()).unwrap();
                resume.lock().unwrap().recv().unwrap();
            }
            Ok(Some(source.clone()))
        })
    });
    let job_a = request(a.path());
    let job_b = request(b.path());
    let bg_scheduler = scheduler.clone();
    let background =
        std::thread::spawn(move || bg_scheduler.load(job_a, Priority::Eager, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let fg_scheduler = scheduler.clone();
    let foreground =
        std::thread::spawn(move || fg_scheduler.load(job_b, Priority::Foreground, || false));
    // Completes while the background step still blocks: it does not queue behind it.
    let developed = foreground.join().unwrap().unwrap();
    assert!(Arc::ptr_eq(&developed, &expected));
    release.send(()).unwrap();
    assert!(background.join().unwrap().is_ok());
}

#[test]
fn background_work_yields_while_foreground_demand_lives() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Arc::new(Mutex::new(resume));
    let scheduler = SourceLoader::start_workers(2, move |_, _| {
        let entered = entered.clone();
        let resume = resume.clone();
        let source = source.clone();
        Box::new(move |task| {
            if task.id == 1 {
                entered.send(()).unwrap();
                resume.lock().unwrap().recv().unwrap();
            }
            Ok(Some(source.clone()))
        })
    });
    let job_a = request(a.path());
    let job_b = request(b.path());
    let fg_scheduler = scheduler.clone();
    let foreground =
        std::thread::spawn(move || fg_scheduler.load(job_a, Priority::Foreground, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let bg_scheduler = scheduler.clone();
    let background =
        std::thread::spawn(move || bg_scheduler.load(job_b, Priority::Eager, || false));
    until(&scheduler, |state| state.tasks.len() == 2);
    // A second owner exists, but no background work starts while the foreground
    // demand lives; a violation would enter the step promptly and set `started`.
    std::thread::sleep(Duration::from_millis(250));
    {
        let state = scheduler.owner.0.state.lock().unwrap();
        let queued = state.tasks.iter().find(|task| task.id == 2).unwrap();
        assert!(
            !queued.started.load(Ordering::Relaxed),
            "background began during foreground demand"
        );
    }
    release.send(()).unwrap();
    assert!(foreground.join().unwrap().is_ok());
    assert!(background.join().unwrap().is_ok());
}

#[test]
fn concurrent_native_development_preserves_each_sources_distinct_pixels() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng_with_rgb([6000, 24000, 36000]);
    let expected_a = Scan::open(a.path(), Some([1.; 4])).unwrap();
    let expected_b = Scan::open(b.path(), Some([1.; 4])).unwrap();
    assert_ne!(
        expected_a.pixel(16, 16).unwrap(),
        expected_b.pixel(16, 16).unwrap()
    );
    let scheduler = SourceLoader::new();
    let sa = scheduler.clone();
    let job_a = request(a.path());
    let sb = scheduler.clone();
    let job_b = request(b.path());
    let first = std::thread::spawn(move || sa.load(job_a, Priority::Eager, || false));
    let second = std::thread::spawn(move || sb.load(job_b, Priority::Eager, || false));
    let source_a = first.join().unwrap().unwrap();
    let source_b = second.join().unwrap().unwrap();
    for (source, expected) in [(&source_a, &expected_a), (&source_b, &expected_b)] {
        let developed = source.camera_image().unwrap();
        let frozen = expected.source().camera_image().unwrap();
        assert_eq!(
            (developed.width, developed.height),
            (frozen.width, frozen.height)
        );
        for y in 0..frozen.height {
            for x in 0..frozen.width {
                assert_eq!(
                    developed.pixel_samples(x, y).unwrap(),
                    frozen.pixel_samples(x, y).unwrap()
                );
            }
        }
    }
}

#[test]
fn closing_the_pair_wakes_waiters_and_stops_both_owners() {
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let source = Scan::open(a.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Arc::new(Mutex::new(resume));
    let scheduler = SourceLoader::start_workers(2, move |_, _| {
        let entered = entered.clone();
        let resume = resume.clone();
        let source = source.clone();
        Box::new(move |_| {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv().unwrap();
            Ok(Some(source.clone()))
        })
    });
    let sa = scheduler.clone();
    let job_a = request(a.path());
    let sb = scheduler.clone();
    let job_b = request(b.path());
    let first = std::thread::spawn(move || sa.load(job_a, Priority::Eager, || false));
    let second = std::thread::spawn(move || sb.load(job_b, Priority::Eager, || false));
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    scheduler.close();
    assert!(first.join().unwrap().is_err());
    assert!(second.join().unwrap().is_err());
    release.send(()).unwrap();
    release.send(()).unwrap();
    scheduler.close_and_wait().unwrap();
}

#[test]
fn sdk_development_holds_native_execution_exclusive_end_to_end() {
    // The direct-SDK route serializes its transactions process-wide in the
    // bridge, so two owners must never develop through it concurrently: an
    // overlap would halve the running call's team while the peer blocks.
    let a = fixture::bayer_dng();
    let b = fixture::bayer_dng();
    let (record, raw) = source_metadata::open_source(a.path(), Some([1.; 4])).unwrap();
    assert!(
        raw.metadata.uses_sdk(),
        "the fixture must exercise the direct SDK route"
    );
    drop(raw);
    drop(record);
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (counted, observed) = (live.clone(), peak.clone());
    let scheduler = SourceLoader::new_observed(move |_, phase, _| {
        match phase {
            NativePhase::DevelopBegin => {
                let now = counted.fetch_add(1, Ordering::SeqCst) + 1;
                observed.fetch_max(now, Ordering::SeqCst);
            }
            NativePhase::DevelopEnd => {
                counted.fetch_sub(1, Ordering::SeqCst);
            }
            _ => {}
        }
        Ok(())
    });
    let mut waiters = Vec::new();
    for file in [&a, &b] {
        let job = request(file.path());
        let client = scheduler.clone();
        waiters.push(std::thread::spawn(move || {
            client.load(job, Priority::Eager, || false)
        }));
    }
    for waiter in waiters {
        // A lost wakeup here parks an owner instead of returning, so the join
        // is bounded by the channel-free scheduler noticing the released step.
        waiter.join().unwrap().unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    assert_eq!(live.load(Ordering::SeqCst), 0);
    scheduler.close_and_wait().unwrap();
}
