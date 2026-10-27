use super::*;
use crate::application::test_support::TestApplication;
use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[test]
fn nested_ui_defers_even_factory_state_access_until_after_the_mutable_ui_borrow() {
    let caption = String::from("captured input");
    let caption = caption.as_str();
    let owner = RefCell::new(Some(0));
    let constructed = Cell::new(false);
    let constructed = &constructed;
    let notifications = Arc::new(AtomicUsize::new(0));
    let observed = notifications.clone();
    let mut app = TestApplication::new(&owner, move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let cx = app.access();
    let (send, receive) = futures_channel::oneshot::channel::<i32>();
    let mut receive = Some(receive);
    let mut task = None;
    let mut output = egui::Context::default().run_ui(Default::default(), |ui| {
        app.update(|model| {
            *model = 1;
            ui.horizontal(|ui| {
                ui.label(caption);
                if let Some(receive) = receive.take() {
                    task = cx.spawn(move |cx| {
                        constructed.set(true);
                        assert_eq!(cx.read(|value| *value), 1);
                        cx.update(|value| *value = 2).unwrap();
                        // Successful no-op updates retain the notification contract.
                        cx.update(|value| assert_eq!(*value, 2)).unwrap();
                        async move {
                            let value = receive.await.unwrap();
                            assert_eq!(caption, "captured input");
                            cx.update(|state| *state += value).unwrap();
                        }
                    });
                }
            });
            assert!(!constructed.get());
            assert_eq!(*model, 1);
        });
    });
    output.textures_delta.clear();
    assert!(task.is_some());
    assert_eq!(notifications.load(Ordering::SeqCst), 0);
    app.pump();
    assert!(constructed.get());
    assert_eq!(app.read(|value| *value), 2);
    assert_eq!(notifications.load(Ordering::SeqCst), 2);
    std::thread::spawn(move || send.send(40).unwrap())
        .join()
        .unwrap();
    app.run_until(Duration::from_secs(5), |value| *value == 42);
    assert_eq!(notifications.load(Ordering::SeqCst), 3);
    drop(task);
}

#[test]
fn supplied_context_can_spawn_and_await_another_workflow_without_capturing_a_second_handle() {
    test_support::with_application(20, |app| {
        let _task = app
            .access()
            .spawn(|cx| async move {
                let child = cx
                    .spawn(|cx| async move { cx.read(|value| *value + 1) })
                    .unwrap();
                let value = child.await;
                cx.update(|state| *state = value * 2).unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(5), |value| *value == 42);
    });
}

struct Dropped(Rc<Cell<bool>>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn cancellation_or_shutdown_drops_unpolled_factory_inputs_without_entering_it() {
    for close in [false, true] {
        let owner = RefCell::new(Some(0));
        let mut app = TestApplication::new(&owner, || {});
        let entered = Rc::new(Cell::new(false));
        let dropped = Rc::new(Cell::new(false));
        let guard = Dropped(dropped.clone());
        let called = entered.clone();
        let task = app
            .access()
            .spawn(move |cx| {
                called.set(true);
                async move {
                    let _guard = guard;
                    cx.update(|_| panic!("cancelled factory ran"));
                }
            })
            .unwrap();
        if close {
            app.close_execution();
            drop(task);
        } else {
            drop(task);
            app.pump();
        }
        assert!(!entered.get());
        assert!(dropped.get());
        assert_eq!(app.read(|value| *value), 0);
    }
}

#[test]
fn context_updates_and_spawns_after_shutdown_do_not_mutate_notify_or_enter_the_factory() {
    let owner = RefCell::new(Some(0));
    let saved = Rc::new(RefCell::new(None));
    let notifications = Arc::new(AtomicUsize::new(0));
    let observed = notifications.clone();
    let mut app = TestApplication::new(&owner, move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });
    let target = saved.clone();
    let cx = app.access();
    let _task = cx
        .spawn(move |cx| async move {
            cx.update(|value| *value = 42).unwrap();
            *target.borrow_mut() = Some(cx);
        })
        .unwrap();
    app.run_until(Duration::from_secs(5), |value| *value == 42);
    app.close_execution();
    let saved = saved.borrow();
    let saved = saved.as_ref().unwrap();
    assert!(saved.update(|_| panic!("retired state mutated")).is_none());
    assert_eq!(saved.read(|value| *value), 42);
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    let dropped = Rc::new(Cell::new(false));
    let guard = Dropped(dropped.clone());
    let entered = Rc::new(Cell::new(false));
    let called = entered.clone();
    assert!(
        cx.spawn(move |_| {
            called.set(true);
            async move {
                let _guard = guard;
            }
        })
        .is_none()
    );
    assert!(!entered.get());
    assert!(dropped.get());
}
