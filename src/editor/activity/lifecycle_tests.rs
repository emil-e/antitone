use super::*;
use futures_lite::future::{block_on, poll_once};

#[test]
fn only_the_scoped_handle_owns_visibility_even_with_live_observer_upgrades() {
    let activities = Activities::default();
    let activity = activities.start(Options::new("Working").modal());
    let reporter = activity.reporter();
    let snapshot = activities.view().unwrap();
    let observing = reporter.entry().unwrap();
    assert!(activities.blocks_workspace());
    drop(activity);
    assert!(activities.view().is_none());
    assert!(!activities.blocks_workspace());
    reporter.progress(1, 2);
    reporter.status("Late update");
    snapshot.request_cancel();
    assert!(reporter.view().is_none());
    assert!(!observing.live.load(Ordering::Acquire));
}

#[test]
fn independent_scopes_cannot_update_cancel_or_retire_each_other() {
    let activities = Activities::default();
    let first_signal = Cancellation::default();
    let first = activities.start(Options::new("First").cancellable(
        "Stop first",
        "Stopping first",
        first_signal.clone(),
    ));
    let stale = first.reporter();
    let second_signal = Cancellation::default();
    let second = activities.start(Options::new("Second").cancellable(
        "Stop second",
        "Stopping second",
        second_signal.clone(),
    ));
    first.progress(1, 4);
    assert!(activities.view().unwrap().measured.is_none());
    drop(first);
    stale.progress(2, 4);
    stale.request_cancel();
    assert!(!first_signal.is_cancelled());
    assert!(!second_signal.is_cancelled());
    assert_eq!(activities.view().unwrap().title, "Second");
    drop(second);
    assert!(activities.view().is_none());
}

#[test]
fn modal_precedence_does_not_destroy_overlapping_local_feedback() {
    let activities = Activities::default();
    let modal = activities.start(Options::new("Transaction").modal());
    let local = activities.start(Options::new("Local"));
    assert_eq!(activities.view().unwrap().title, "Transaction");
    drop(modal);
    assert_eq!(activities.view().unwrap().title, "Local");
    assert!(!activities.blocks_workspace());
    drop(local);
    assert!(activities.view().is_none());
}

#[test]
fn phase_completion_and_zero_totals_remain_indeterminate_until_scope_exit() {
    let activities = Activities::default();
    let activity = activities.start(Options::new("Working"));
    activity.progress(1, 4);
    assert_eq!(
        activities.view().unwrap().measured,
        Some((0.25, "1 of 4".into()))
    );
    for (done, total) in [(4, 4), (5, 4), (0, 0)] {
        activity.progress(done, total);
        assert!(activities.view().unwrap().measured.is_none());
    }
    activity.status("Validating");
    assert_eq!(activities.view().unwrap().title, "Validating");
    drop(activity);
    assert!(activities.view().is_none());
}

#[test]
fn cancel_wakes_the_workflow_and_keeps_feedback_until_acknowledgement() {
    block_on(async {
        let activities = Activities::default();
        let signal = Cancellation::default();
        let activity = activities.start(Options::new("Working").modal().cancellable(
            "Stop",
            "Stopping",
            signal.clone(),
        ));
        let mut listening = Box::pin(activity.cancelled());
        assert!(poll_once(&mut listening).await.is_none());
        activities.view().unwrap().request_cancel();
        assert_eq!(poll_once(&mut listening).await, Some(()));
        drop(listening);
        activity.progress(1, 2);
        activity.status("Late worker status");
        let view = activities.view().unwrap();
        assert_eq!(view.title, "Stopping");
        assert!(view.cancel.is_none() && view.measured.is_none());
        assert!(activities.blocks_workspace());
        drop(activity);
        assert!(!activities.blocks_workspace());
    });
}

#[test]
fn unpolled_and_suspended_futures_release_their_activities_on_drop() {
    block_on(async {
        let activities = Activities::default();
        for poll in [false, true] {
            let activity = activities.start(Options::new("Working"));
            let mut future = Box::pin(async move {
                let _activity = activity;
                std::future::pending::<()>().await;
            });
            if poll {
                assert!(poll_once(&mut future).await.is_none());
            }
            assert!(activities.view().is_some());
            drop(future);
            assert!(activities.view().is_none());
        }
    });
}

#[test]
fn updates_and_retirement_request_repaint_without_borrowing_the_ui_owner() {
    use std::{cell::RefCell, sync::atomic::AtomicUsize};
    let context = egui::Context::default();
    let repaints = Arc::new(AtomicUsize::new(0));
    let wake = repaints.clone();
    context.set_request_repaint_callback(move |_| {
        wake.fetch_add(1, Ordering::Relaxed);
    });
    let owner = RefCell::new(Activities::default());
    owner.borrow().set_context(&context);
    let activity = owner.borrow().start(Options::new("Working"));
    let _borrow = owner.borrow_mut();
    let frame = || {
        let mut output = context.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
    };
    for _ in 0..4 {
        frame();
    }
    let before = repaints.load(Ordering::Relaxed);
    activity.status("Updated");
    assert!(repaints.load(Ordering::Relaxed) > before);
    for _ in 0..4 {
        frame();
    }
    let before = repaints.load(Ordering::Relaxed);
    drop(activity);
    assert!(repaints.load(Ordering::Relaxed) > before);
}
