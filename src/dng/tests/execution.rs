use super::*;

fn large_fixture() -> Fixture {
    let mut fixture = Fixture {
        // An odd active extent crosses SDK tiles and leaves partial worker stripes.
        side: 1031,
        ramp: true,
        marker: Some((200, 200, 0)),
        ..Default::default()
    };
    fixture.lists[0].push(Opcode::bad_pixels(0, 2));
    fixture.lists[1].push(Opcode::gain(0.5, 1));
    fixture.lists[2].push(warp(true));
    fixture.lists[2].push(Opcode::warp(&[[1.001, 0., 0., 0., 0., 0.]], [0.5, 0.5]));
    fixture
}

fn large_options() -> Options {
    Options {
        max_pixels: 2_000_000,
        fixed_wb: Some([2., 1., 1.5]),
    }
}

#[test]
fn execution_budgets_preserve_every_camera_bit_and_metadata_across_partial_tiles() {
    let file = large_fixture().write();
    let baseline = Source::develop(file.path(), large_options(), &Cancellation::default()).unwrap();
    for threads in [2, 4, 8] {
        let actual = Source::develop_with_threads(
            file.path(),
            large_options(),
            &Cancellation::default(),
            threads,
        )
        .unwrap();
        assert_eq!(actual.header(), baseline.header());
        assert_eq!(actual.opcode_counts(), baseline.opcode_counts());
        assert!(
            actual
                .camera_rgb()
                .iter()
                .flat_map(|rgb| rgb.map(f32::to_bits))
                .eq(baseline
                    .camera_rgb()
                    .iter()
                    .flat_map(|rgb| rgb.map(f32::to_bits))),
            "camera bits differ at execution budget {threads}"
        );
        let timing = actual.timings();
        // Ensure correspondence actually covers parallel execution, not a small-task serial fallback.
        assert!(timing.sdk_workers > 1 && timing.sdk_workers <= threads);
        assert!(timing.warp_tile_scratch_bytes <= 64 * 1024 * 1024);
    }
}

#[test]
fn large_warps_use_sdk_border_extension_at_each_execution_budget() {
    let mut fixture = Fixture {
        side: 1031,
        ..Default::default()
    };
    fixture.lists[2].push(Opcode::warp(&[[1.05, 0., 0., 0., 0., 0.]], [0.5, 0.5]));
    let file = fixture.write();
    for threads in [1, 8] {
        let source = Source::develop_with_threads(
            file.path(),
            large_options(),
            &Cancellation::default(),
            threads,
        )
        .unwrap();
        // Border extension must preserve this spatially constant camera image.
        for rgb in source.camera_rgb() {
            for (actual, expected) in rgb.iter().zip([0.5, 0.375, 0.75]) {
                assert!((actual - expected).abs() < 1e-6);
            }
        }
        if threads == 8 {
            assert!(source.timings().sdk_workers > 1);
        }
    }
}

#[test]
fn small_sources_keep_serial_execution_and_invalid_budgets_reject() {
    let file = Fixture::default().write();
    let source =
        Source::develop_with_threads(file.path(), options(), &Cancellation::default(), 8).unwrap();
    assert_eq!(source.timings().sdk_workers, 1);
    for threads in [0, 9, u32::MAX] {
        assert!(
            Source::develop_with_threads(file.path(), options(), &Cancellation::default(), threads)
                .is_err()
        );
    }
}

#[test]
fn cancellation_from_a_parallel_worker_returns_no_source_and_releases_the_sdk_job() {
    struct CancelWorker {
        caller: std::thread::ThreadId,
        calls: AtomicUsize,
    }
    extern "C" fn stop(context: *const c_void) -> c_int {
        let state = unsafe { &*context.cast::<CancelWorker>() };
        if std::thread::current().id() != state.caller {
            state.calls.fetch_add(1, Ordering::Relaxed);
            1
        } else {
            0
        }
    }
    let file = large_fixture().write();
    let state = CancelWorker {
        caller: std::thread::current().id(),
        calls: AtomicUsize::new(0),
    };
    let token = Cancellation::default();
    let (path, mut native) = native_request(file.path(), large_options(), &token, 8).unwrap();
    native.cancelled = stop;
    native.cancel_context = (&state as *const CancelWorker).cast();
    assert!(matches!(
        Source::call_native(&path, &native),
        Err(Error::Cancelled)
    ));
    assert!(state.calls.load(Ordering::Relaxed) > 0);
    // No failed worker may keep the process-wide SDK lock or a borrowed callback alive.
    let small = Fixture::default().write();
    assert!(Source::develop(small.path(), options(), &Cancellation::default()).is_ok());
}
