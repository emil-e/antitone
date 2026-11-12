use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    raw::Rect,
};
use crate::{
    cancellation::Cancellation,
    editor::measurement::{self, capture_scan, testing},
};
use async_executor::Task;
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const LIMIT: Duration = Duration::from_secs(5);

fn region(editor: &Editor) -> Coverage {
    let area = editor.selected_area().unwrap();
    Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 24,
    })
}

// Only the synchronous worker is supplied by the fixture. UI registration, execution,
// cancellation and typed result delivery use the same async functions as real features.
fn launch<T: Send + 'static>(
    app: &TestApplication<'_, Editor>,
    display: Display,
    work: impl FnOnce(Coverage, Cancellation) -> Result<T, Error> + Send + 'static,
) -> Task<Option<T>> {
    app.access()
        .spawn(move |cx| async move {
            let worker = cx.clone();
            let region = pick(&cx, display).await?;
            measurement::run(&cx, display.label, async move {
                measurement::query(&worker, move |cancelled| work(region, cancelled)).await
            })
            .await
        })
        .unwrap()
}

fn submit(app: &TestApplication<'_, Editor>) {
    app.enter(|editor, cx| {
        editor.set_test_pick_region(Some(region(editor)));
        editor.apply_pick(cx);
    });
}

#[test]
fn an_unlisted_caller_awaits_the_whole_interaction_and_receives_its_own_value_type() {
    #[derive(Debug, PartialEq)]
    struct Statistics {
        mean: glam::DVec3,
        area: u32,
    }
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let requested = region(&editor);
    let expected = capture_scan(&editor)
        .unwrap()
        .reference_coverage(requested)
        .unwrap();
    let before = editor.document.project.clone();
    let mut image = capture_scan(&editor).unwrap();
    let called = Arc::new(AtomicBool::new(false));
    with_application(editor, |app| {
        let sampled = called.clone();
        let task = launch(
            app,
            Display::sample("Custom camera statistics", View::Source),
            move |region, _| {
                sampled.store(true, Ordering::Relaxed);
                let bounds = region.exact_rectangle(image.area).unwrap();
                Ok(Statistics {
                    mean: image.reference_coverage(region)?,
                    area: bounds.width * bounds.height,
                })
            },
        );
        app.pump();
        assert!(!task.is_finished() && !called.load(Ordering::Relaxed));
        app.read(|editor| {
            assert_eq!(
                editor.test_pick().unwrap().label,
                "Custom camera statistics"
            );
            assert!(!editor.interaction_blocked());
        });
        submit(app);
        app.run_until(LIMIT, |editor| task.is_finished() && !editor.picking());
        assert_eq!(
            pollster::block_on(task),
            Some(Statistics {
                mean: expected,
                area: 768
            })
        );
        assert!(called.load(Ordering::Relaxed));
        app.read(|editor| assert_eq!(editor.document.project, before));
    });
}

#[test]
fn the_executor_drives_the_owned_caller_without_polling_its_join_handle() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let called = Arc::new(AtomicBool::new(false));
    with_application(editor, |app| {
        let sampled = called.clone();
        let task = launch(
            app,
            Display::sample("Independent measurement", View::Source),
            move |_, _| {
                sampled.store(true, Ordering::Relaxed);
                Ok(42)
            },
        );
        app.pump();
        submit(app);
        app.run_until(LIMIT, |editor| task.is_finished() && !editor.picking());
        assert!(called.load(Ordering::Relaxed));
        assert_eq!(pollster::block_on(task), Some(42));
    });
}

#[test]
fn one_caller_can_await_two_pickers_in_sequence_without_an_intervening_yield() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let region_result = region(&editor);
    with_application(editor, |app| {
        let task = app
            .access()
            .spawn(move |cx| async move {
                let first = pick(&cx, Display::sample("First reading", View::Source)).await?;
                let second = pick(&cx, Display::sample("Second reading", View::Working)).await?;
                Some((first, second))
            })
            .unwrap();
        app.pump();
        app.read(|editor| assert_eq!(editor.test_pick().unwrap().label, "First reading"));
        submit(app);
        app.run_until(LIMIT, |editor| {
            task.is_finished()
                || editor
                    .test_pick()
                    .is_some_and(|display| display.label == "Second reading")
        });
        assert!(
            !task.is_finished(),
            "the second await must open another picker, not reject a leftover registration"
        );
        app.read(|editor| assert_eq!(editor.test_pick().unwrap().label, "Second reading"));
        submit(app);
        app.run_until(LIMIT, |_| task.is_finished());
        assert_eq!(
            pollster::block_on(task),
            Some((region_result, region_result))
        );
        app.read(|editor| assert!(!editor.picking() && editor.activities.view().is_none()));
    });
}

#[test]
fn apply_returns_coverage_and_restores_the_view_without_starting_an_activity() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let before = app.read(|editor| {
            (
                editor.document.source,
                editor.document.visit.canvas.viewport,
            )
        });
        let cx = app.access();
        let mut selection = Box::pin(pick(&cx, Display::sample("Region only", View::Source)));
        assert!(
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut selection))
                .is_none()
        );
        let expected = app.read(region);
        submit(app);
        // The receiver has not resumed yet: Apply itself must dismiss the tool.
        app.read(|editor| {
            assert!(!editor.has_hosted_tool());
            assert!(!editor.interaction_blocked() && editor.activities.view().is_none());
            assert_eq!(
                (
                    editor.document.source,
                    editor.document.visit.canvas.viewport
                ),
                before
            );
        });
        assert_eq!(pollster::block_on(selection), Some(expected));
    });
}

#[test]
fn an_unpolled_picker_is_lazy_and_abandoning_it_does_not_register_ui() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let cx = app.access();
        let reading = pick(&cx, Display::sample("Never opened", View::Source));
        app.read(|editor| assert!(!editor.picking()));
        app.update(|_| drop(reading));
        app.pump();
        app.read(|editor| assert!(!editor.has_hosted_tool()));
    });
}

#[test]
fn dropping_a_registered_picker_inside_a_model_borrow_defers_its_ui_cleanup() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let cx = app.access();
        let mut reading = Box::pin(pick(&cx, Display::sample("Dropped reading", View::Source)));
        assert!(
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut reading)).is_none()
        );
        app.read(|editor| assert!(editor.picking()));
        app.update(|_| drop(reading));
        app.run_until(LIMIT, |editor| !editor.picking());
        app.read(|editor| assert!(!editor.has_hosted_tool() && !editor.interaction_blocked()));
    });
}

#[test]
fn abandoned_registration_cleanup_cannot_withdraw_a_successor() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let cx = app.access();
        let mut reading = Box::pin(pick(&cx, Display::sample("Old reading", View::Source)));
        assert!(
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut reading)).is_none()
        );
        app.update(|editor| {
            drop(reading);
            editor.begin_crop();
        });
        app.pump();
        app.read(|editor| assert!(editor.cropping() && !editor.picking()));
    });
}

#[test]
fn a_late_first_poll_does_not_replace_a_newer_tool() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let task = launch(
            app,
            Display::sample("Late reading", View::Source),
            |_, _| Ok(42),
        );
        app.update(Editor::begin_crop);
        app.run_until(LIMIT, |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        app.read(|editor| assert!(editor.cropping()));
    });
}

#[test]
fn a_late_first_poll_refuses_reserved_input_without_replaying_after_release() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let task = launch(
            app,
            Display::sample("Late reading", View::Source),
            |_, _| Ok(42),
        );
        let block = app.update(|editor| editor.workspace_input.block());
        app.run_until(LIMIT, |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        app.read(|editor| assert!(!editor.picking()));

        drop(block);
        app.pump();
        app.read(|editor| assert!(!editor.picking(), "a refused picker is not replayed"));
        let retry = launch(
            app,
            Display::sample("Fresh reading", View::Source),
            |_, _| Ok(42),
        );
        app.pump();
        app.read(|editor| assert!(editor.picking()));
        app.enter(|editor, cx| editor.finish_pick(cx));
        app.run_until(LIMIT, |_| retry.is_finished());
        assert_eq!(pollster::block_on(retry), None);
    });
}

#[test]
fn replacing_an_unpolled_action_drops_the_callers_continuation() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let continued = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        let resumed = continued.clone();
        app.enter(|editor, cx| {
            editor
                .start_interaction(cx, move |cx| async move {
                    let _ = pick(&cx, Display::sample("Retired action", View::Source)).await;
                    resumed.set(true);
                })
                .unwrap();
            editor.begin_crop();
        });
        app.pump();
        assert!(!continued.get());
        app.read(|editor| assert!(editor.cropping() && !editor.picking()));
    });
}

#[test]
fn cancellation_before_submission_returns_no_value_without_running_measurement() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let called = Arc::new(AtomicBool::new(false));
    with_application(editor, |app| {
        let sampled = called.clone();
        let task = launch(
            app,
            Display::sample("Cancelled selection", View::Source),
            move |_, _| {
                sampled.store(true, Ordering::Relaxed);
                Ok(42)
            },
        );
        app.pump();
        app.enter(|editor, cx| editor.finish_pick(cx));
        app.run_until(LIMIT, |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        assert!(!called.load(Ordering::Relaxed));
    });
}

#[test]
fn cancellation_after_submission_but_before_dispatch_does_not_admit_measurement() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let called = Arc::new(AtomicBool::new(false));
    with_application(editor, |app| {
        let sampled = called.clone();
        let task = launch(
            app,
            Display::sample("Withdrawn submission", View::Source),
            move |_, _| {
                sampled.store(true, Ordering::Relaxed);
                Ok(42)
            },
        );
        app.pump();
        submit(app);
        app.update(|_| drop(task));
        app.run_until(LIMIT, |editor| !editor.picking());
        assert!(!called.load(Ordering::Relaxed));
        app.read(|editor| {
            assert!(!editor.interaction_blocked() && editor.activities.view().is_none())
        });
    });
}

#[test]
fn retirement_withdraws_running_work_without_releasing_its_physical_inputs() {
    for drop_caller in [false, true] {
        let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
        editor.document.cache.set_budget(0);
        let mut image = capture_scan(&editor).unwrap();
        let source = Arc::downgrade(image.source());
        let before = editor.document.project.clone();
        let gate = testing::Gate::install(&mut editor);
        let resumed = Rc::new(Cell::new(false));
        with_application(editor, |app| {
            let continued = resumed.clone();
            let worker_cx = app.access();
            let task = app
                .access()
                .spawn(move |cx| async move {
                    let region =
                        pick(&cx, Display::sample("Held measurement", View::Source)).await?;
                    let value = measurement::run(&cx, "Held measurement", async move {
                        measurement::query(&worker_cx, move |_| image.reference_coverage(region))
                            .await
                    })
                    .await;
                    continued.set(true);
                    value
                })
                .unwrap();
            app.pump();
            submit(app);
            let call = gate.next(app);
            if drop_caller {
                app.update(|_| drop(task));
            } else {
                app.update(|editor| editor.activities.view().unwrap().request_cancel());
                app.run_until(LIMIT, |_| task.is_finished());
                assert_eq!(pollster::block_on(task), None);
            }
            app.run_until(LIMIT, |editor| !editor.measurement_pending());
            assert!(call.cancelled.is_cancelled());
            assert!(source.upgrade().is_some());
            assert_eq!(resumed.get(), !drop_caller);
            app.update(|editor| {
                editor.document.visit.canvas.ready = None;
                editor.begin_crop();
            });
            let completed = call.completed.clone();
            call.run();
            completed.wait(app);
            // Retired physical work has no UI continuation to wake the application.
            let deadline = std::time::Instant::now() + LIMIT;
            while source.upgrade().is_some() {
                app.pump();
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker kept its input (drop caller: {drop_caller})"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            app.read(|editor| {
                assert!(editor.cropping());
                assert!(editor.activities.view().is_none());
                assert_eq!(editor.document.project, before);
            });
        });
    }
}

#[test]
fn errors_and_worker_panics_return_no_value_without_stranding_the_ui() {
    for panic in [false, true] {
        let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
        with_application(editor, |app| {
            let task = launch(
                app,
                Display::sample("Failed measurement", View::Source),
                move |_, _| {
                    assert!(!panic, "injected worker panic");
                    Err::<u32, _>(Error::UnusableSample("test measurement rejected"))
                },
            );
            app.pump();
            submit(app);
            app.run_until(LIMIT, |editor| task.is_finished() && !editor.picking());
            assert_eq!(pollster::block_on(task), None);
            app.read(|editor| {
                assert!(!editor.interaction_blocked());
                assert!(editor.activities.view().is_none());
                assert!(
                    editor.status.contains(if panic {
                        "panicked"
                    } else {
                        "test measurement rejected"
                    }),
                    "{}",
                    editor.status
                );
            });
        });
    }
}

#[test]
fn photo_retirement_cancels_the_picker_but_not_an_independently_owned_caller() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let task = launch(
            app,
            Display::sample("Visit reading", View::Source),
            |_, _| Ok(42),
        );
        app.pump();
        app.update(|editor| editor.replace_photo_scope(editor.id));
        app.run_until(LIMIT, |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        app.read(|editor| assert!(!editor.picking() && !editor.interaction_blocked()));
    });
}

#[test]
fn a_callers_continuation_outlives_the_picker_and_can_install_its_own_successor() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let received = Rc::new(Cell::new(false));
    let applied = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        let (release, wait) = oneshot::channel::<()>();
        let got = received.clone();
        let done = applied.clone();
        let worker_cx = app.access();
        app.enter(|editor, cx| {
            editor
                .start_interaction(cx, move |cx| async move {
                    let Some(region) =
                        pick(&cx, Display::sample("Caller reading", View::Working)).await
                    else {
                        return;
                    };
                    let value = measurement::run(&cx, "Caller reading", async move {
                        measurement::query(&worker_cx, move |_| Ok(region.corners().len())).await
                    })
                    .await;
                    assert_eq!(value, Some(4));
                    got.set(true);
                    if wait.await.is_err() {
                        return;
                    }
                    cx.update(|editor| {
                        editor.begin_crop();
                        done.set(true);
                    });
                })
                .unwrap();
        });
        app.pump();
        submit(app);
        app.run_until(LIMIT, |editor| received.get() && !editor.picking());
        assert!(!applied.get());
        release.send(()).unwrap();
        app.run_until(LIMIT, |_| applied.get());
        app.read(|editor| assert!(editor.cropping()));
    });
}
