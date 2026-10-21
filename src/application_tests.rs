use super::*;
use futures_channel::oneshot;
use std::{
    cell::Cell,
    sync::{Mutex, mpsc},
    time::Duration,
};

fn application<'scope>() -> (Application<'scope>, mpsc::Receiver<()>, Arc<Mutex<usize>>) {
    let (wake, wakes) = mpsc::channel();
    let paints = Arc::new(Mutex::new(0));
    let repaint = paints.clone();
    let application = Application::new(
        move || {
            let _ = wake.send(());
        },
        move || *repaint.lock().unwrap() += 1,
    );
    (application, wakes, paints)
}

fn turn(application: &mut Application<'_>, wakes: &mpsc::Receiver<()>) {
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    application.drive();
}

#[test]
fn a_borrowed_non_send_workflow_resumes_without_a_ui_frame() {
    let state = RefCell::new(Some(None));
    let resumed = Cell::new(false);
    let (mut application, wakes, paints) = application();
    let access = application.access(&state);
    let ui_thread = std::thread::current().id();
    let (send, receive) = oneshot::channel();
    let target = access.clone();
    let resumed_ref = &resumed;
    let _task = access
        .spawn(async move {
            assert_eq!(std::thread::current().id(), ui_thread);
            let value = receive.await.unwrap();
            assert_eq!(std::thread::current().id(), ui_thread);
            resumed_ref.set(true);
            target.update(|state| *state = Some(value)).unwrap();
        })
        .unwrap();
    turn(&mut application, &wakes);
    assert!(!resumed.get());
    assert!(wakes.try_recv().is_err(), "a suspended task must not spin");
    // Ordinary access remains available while the future is suspended.
    access.with_mut(|state| assert!(state.is_none()));
    std::thread::spawn(move || send.send(42).unwrap())
        .join()
        .unwrap();
    assert_eq!(access.read(|state| *state), None);
    assert_eq!(*paints.lock().unwrap(), 0);
    turn(&mut application, &wakes);
    assert!(resumed.get());
    assert_eq!(access.read(|state| *state), Some(42));
    assert_eq!(*paints.lock().unwrap(), 1);
}

struct OnDrop<'a>(&'a Cell<bool>);
impl Drop for OnDrop<'_> {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn cancelling_a_scoped_task_destroys_its_borrowed_inputs_without_updating_state() {
    let dropped = Cell::new(false);
    let state = RefCell::new(Some(0));
    let (mut application, wakes, _) = application();
    let access = application.access(&state);
    let target = access.clone();
    let guard = OnDrop(&dropped);
    let task = access
        .spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
            target.update(|value| *value = 1);
        })
        .unwrap();
    turn(&mut application, &wakes);
    drop(task);
    turn(&mut application, &wakes);
    assert!(dropped.get());
    assert_eq!(access.read(|value| *value), 0);
}

#[test]
fn shutdown_cancels_borrowed_futures_before_state_teardown_and_rejects_new_work() {
    let state_dropped = Cell::new(false);
    let future_dropped = Cell::new(false);
    let state = RefCell::new(Some(OnDrop(&state_dropped)));
    let (mut application, wakes, _) = application();
    let access = application.access(&state);
    let guard = OnDrop(&future_dropped);
    let (send, receive) = oneshot::channel::<()>();
    let _task = access
        .spawn(async move {
            let _guard = guard;
            let _ = receive.await;
        })
        .unwrap();
    turn(&mut application, &wakes);
    application.close();
    assert!(future_dropped.get());
    assert!(!state_dropped.get());
    assert!(access.update(|_| ()).is_none());
    assert!(access.spawn(async {}).is_none());
    drop(state.borrow_mut().take());
    assert!(state_dropped.get());
    assert!(
        std::thread::spawn(move || send.send(()))
            .join()
            .unwrap()
            .is_err()
    );
    application.drive();
}

#[test]
fn library_tasks_can_be_awaited_directly_without_a_result_forwarding_channel() {
    let state = RefCell::new(Some(0));
    let (mut application, wakes, _) = application();
    let access = application.access(&state);
    let result = access.spawn(async { 21 }).unwrap();
    let target = access.clone();
    let _workflow = access
        .spawn(async move {
            let value = result.await;
            target.update(|state| *state = value * 2).unwrap();
        })
        .unwrap();
    turn(&mut application, &wakes);
    assert_eq!(access.read(|state| *state), 42);
}

#[test]
fn retiring_before_initialization_drops_unpolled_borrowed_work() {
    let dropped = Cell::new(false);
    let state = RefCell::new(None::<()>);
    let (mut application, _, _) = application();
    let access = application.access(&state);
    let guard = OnDrop(&dropped);
    let target = access.clone();
    let _task = access
        .spawn(async move {
            let _guard = guard;
            target.update(|_| panic!("uninitialized application was accessed"));
        })
        .unwrap();
    application.close();
    assert!(dropped.get());
    assert!(state.borrow().is_none());
    application.drive();
}

#[test]
fn short_update_borrows_are_released_when_a_callback_unwinds() {
    let state = RefCell::new(Some(0));
    let (application, _, _) = application();
    let access = application.access(&state);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        access.update(|value| {
            *value = 1;
            panic!("update failed")
        });
    }));
    assert!(result.is_err());
    access.update(|value| *value += 1).unwrap();
    assert_eq!(access.read(|value| *value), 2);
}
