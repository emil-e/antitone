use super::*;
use crate::{
    application::test_support::with_application,
    editor::picking::{Display, View},
    raw::Rect,
};
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn region(editor: &Editor) -> Coverage {
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 24,
    })
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
    let before = editor.document.state.project.clone();
    let called = Arc::new(AtomicBool::new(false));
    let consumed_area = Rc::new(Cell::new(None));
    with_application(editor, |app| {
        let sampled = called.clone();
        let consumed = consumed_area.clone();
        let task = app.enter(|editor, cx| {
            let mut image = capture_scan(editor).unwrap();
            let value = pick(
                Display::sample("Custom camera statistics", View::Source),
                move |region, _| {
                    sampled.store(true, Ordering::Relaxed);
                    let bounds = region.exact_rectangle(image.area).unwrap();
                    Ok(Statistics {
                        mean: image.reference_coverage(region)?,
                        area: bounds.width * bounds.height,
                    })
                },
                cx.clone(),
            );
            cx.spawn(move |_| async move {
                let value = value.await?;
                consumed.set(Some(value.area));
                Some(value)
            })
            .unwrap()
        });
        app.pump();
        assert!(!task.is_finished());
        assert!(!called.load(Ordering::Relaxed));
        assert_eq!(consumed_area.get(), None);
        app.read(|editor| {
            assert_eq!(
                editor.active_pick().unwrap().label,
                "Custom camera statistics"
            );
            assert!(!editor.busy && !editor.measurement_pending());
            assert_eq!(editor.document.state.project, before);
        });
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(requested));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |_| task.is_finished());
        assert_eq!(
            pollster::block_on(task),
            Some(Statistics {
                mean: expected,
                area: 768
            })
        );
        assert!(called.load(Ordering::Relaxed));
        assert_eq!(consumed_area.get(), Some(768));
        app.read(|editor| {
            assert!(editor.active_pick().is_none() && !editor.busy);
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn discarding_an_unpolled_pick_inside_a_state_transaction_safely_withdraws_it() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Unused reading", View::Source),
                |_, _| Ok(42),
                cx.clone(),
            );
            drop(value);
        });
        app.pump();
        assert!(app.read(|editor| editor.active_pick().is_none() && !editor.busy));
    });
}

#[test]
fn dropping_a_polled_pick_inside_a_state_transaction_defers_its_ui_cleanup() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let mut value = Box::pin(pick(
            Display::sample("Dropped reading", View::Source),
            |_, _| Ok(42),
            app.access(),
        ));
        assert!(
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut value)).is_none()
        );
        assert!(app.read(|editor| editor.active_pick().is_some()));
        app.update(|_| drop(value));
        app.pump();
        assert!(app.read(|editor| editor.active_pick().is_none() && !editor.busy));
    });
}

#[test]
fn dropping_the_caller_during_measurement_withdraws_ui_but_retains_running_inputs() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.document.cache.set_budget(0);
    let requested = region(&editor);
    let mut image = capture_scan(&editor).unwrap();
    let source = Arc::downgrade(image.source());
    let before = editor.document.state.project.clone();
    let gate = testing::Gate::install(&mut editor);
    let resumed = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        let continued = resumed.clone();
        let task = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Abandoned measurement", View::Source),
                move |region, _| image.reference_coverage(region),
                cx.clone(),
            );
            cx.spawn(move |_| async move {
                let value = value.await;
                continued.set(true);
                value
            })
            .unwrap()
        });
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(requested));
            editor.apply_pick(cx);
        });
        let call = gate.next(app);
        app.update(|editor| {
            drop(task);
            editor.document.visit.canvas.ready = None;
        });
        app.pump();
        assert!(call.cancelled.is_cancelled());
        assert!(source.upgrade().is_some());
        assert!(!resumed.get());
        app.read(|editor| {
            assert!(editor.active_pick().is_none() && !editor.busy);
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.state.project, before);
        });
        call.run();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while source.upgrade().is_some() {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "worker kept its input"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!resumed.get());
        app.read(|editor| {
            assert!(editor.active_pick().is_none() && !editor.busy);
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn cancelling_selection_resumes_the_caller_without_a_value_or_running_measurement() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let called = Arc::new(AtomicBool::new(false));
    with_application(editor, |app| {
        let sampled = called.clone();
        let task = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Optional reading", View::Source),
                move |_, _| {
                    sampled.store(true, Ordering::Relaxed);
                    Ok(42)
                },
                cx.clone(),
            );
            cx.spawn(move |_| value).unwrap()
        });
        app.pump();
        app.enter(|editor, cx| editor.finish_pick(cx));
        app.run_until(Duration::from_secs(5), |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        assert!(!called.load(Ordering::Relaxed));
        assert!(app.read(|editor| editor.active_pick().is_none() && !editor.busy));
    });
}

#[test]
fn measurement_failures_are_presented_and_the_caller_only_needs_to_handle_no_value() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        let task = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Fallible reading", View::Source),
                |_, _| Err::<f64, _>(Error::UnusableSample("declared measurement failure")),
                cx.clone(),
            );
            cx.spawn(move |_| value).unwrap()
        });
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region(editor)));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        app.read(|editor| {
            assert!(editor.status.contains("declared measurement failure"));
            assert!(editor.active_pick().is_none() && !editor.busy);
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn dropping_an_unpolled_caller_withdraws_the_interaction_without_resuming_it() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let resumed = Rc::new(Cell::new(false));
    let called = Arc::new(AtomicBool::new(false));
    with_application(editor, |app| {
        let continued = resumed.clone();
        let sampled = called.clone();
        let task = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Abandoned reading", View::Source),
                move |_, _| {
                    sampled.store(true, Ordering::Relaxed);
                    Ok(42)
                },
                cx.clone(),
            );
            cx.spawn(move |_| async move {
                let _ = value.await;
                continued.set(true);
            })
            .unwrap()
        });
        drop(task);
        app.pump();
        assert!(!resumed.get() && !called.load(Ordering::Relaxed));
        assert!(app.read(|editor| editor.active_pick().is_none() && !editor.busy));
    });
}

#[test]
fn retiring_the_display_visit_cancels_an_independently_owned_waiter_too() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let task = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Visit reading", View::Source),
                |_, _| Ok(42),
                cx.clone(),
            );
            cx.spawn(move |_| value).unwrap()
        });
        app.pump();
        app.update(|editor| {
            let next = editor.request_id();
            editor.replace_photo_scope(next);
        });
        app.run_until(Duration::from_secs(5), |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        assert!(app.read(|editor| editor.active_pick().is_none()));
    });
}

#[test]
fn a_cancelled_callers_cleanup_cannot_withdraw_the_next_interaction() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    with_application(editor, |app| {
        let first = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("First reading", View::Source),
                |_, _| Ok(1),
                cx.clone(),
            );
            cx.spawn(move |_| value).unwrap()
        });
        app.pump();
        assert_eq!(
            app.read(|editor| editor.active_pick().unwrap().label),
            "First reading"
        );
        let next = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Next reading", View::Source),
                |_, _| Ok(2),
                cx.clone(),
            );
            cx.spawn(move |_| value).unwrap()
        });
        app.pump();
        assert!(first.is_finished());
        assert_eq!(pollster::block_on(first), None);
        assert!(!next.is_finished());
        assert_eq!(
            app.read(|editor| editor.active_pick().unwrap().label),
            "Next reading"
        );
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region(editor)));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |_| next.is_finished());
        assert_eq!(pollster::block_on(next), Some(2));
    });
}

#[test]
fn a_worker_panic_yields_no_value_and_does_not_strand_the_interaction() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let gate = testing::Gate::install(&mut editor);
    with_application(editor, |app| {
        let task = app.enter(|_editor, cx| {
            let value = pick(
                Display::sample("Panicking reading", View::Source),
                |_, _| Ok(42),
                cx.clone(),
            );
            cx.spawn(move |_| value).unwrap()
        });
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region(editor)));
            editor.apply_pick(cx);
        });
        gate.next(app).panic();
        app.run_until(Duration::from_secs(5), |_| task.is_finished());
        assert_eq!(pollster::block_on(task), None);
        app.read(|editor| {
            assert!(editor.status.contains("panicked"));
            assert!(editor.active_pick().is_none() && !editor.busy);
        });
    });
}
