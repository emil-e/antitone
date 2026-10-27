//! A live host substitute: state transactions and wake delivery are separate
//! from painting, and state storage outlives the borrowing executor.
use super::{Application, UiContext};
use std::{
    cell::RefCell,
    sync::mpsc,
    time::{Duration, Instant},
};

pub(crate) struct TestApplication<'scope, T> {
    access: UiContext<'scope, T>,
    application: Application<'scope>,
    wakes: mpsc::Receiver<()>,
}

pub(crate) fn with_application<T, R>(
    value: T,
    test: impl FnOnce(&mut TestApplication<'_, T>) -> R,
) -> R {
    let owner = RefCell::new(Some(value));
    let mut application = TestApplication::new(&owner, || {});
    test(&mut application)
}

impl<'scope, T: 'scope> TestApplication<'scope, T> {
    pub fn new(
        owner: &'scope RefCell<Option<T>>,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (wake, wakes) = mpsc::channel();
        let application = Application::new(
            move || {
                let _ = wake.send(());
            },
            repaint,
        );
        let access = application.access(owner);
        Self {
            access,
            application,
            wakes,
        }
    }

    pub fn access(&self) -> UiContext<'scope, T> {
        self.access.clone()
    }

    pub fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        self.access.read(read)
    }

    pub fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
        self.access.with_mut(update)
    }

    pub fn enter<R>(&self, input: impl FnOnce(&mut T, &UiContext<'scope, T>) -> R) -> R {
        self.access.with_mut(|state| input(state, &self.access))
    }

    /// Deliver one cooperative host turn, coalescing notifications already queued.
    /// A self-waking task gets a later turn rather than monopolizing this call.
    pub fn pump(&mut self) {
        let requested = self.wakes.try_iter().count() != 0;
        if requested {
            self.application.drive();
        }
    }

    /// Await executor notifications without rendering or repeatedly polling state.
    pub fn run_until(&mut self, timeout: Duration, done: impl Fn(&T) -> bool) {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump();
            if self.read(&done) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "application did not reach the expected state before its deadline"
            );
            self.wakes
                .recv_timeout(remaining)
                .expect("application did not reach the expected state before its deadline");
            self.application.drive();
        }
    }

    pub fn close_execution(&mut self) {
        self.application.close();
    }
}

impl<T> Drop for TestApplication<'_, T> {
    fn drop(&mut self) {
        self.application.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct FutureDrop(Rc<Cell<bool>>);

    impl Drop for FutureDrop {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    struct StateDrop {
        future: Rc<Cell<bool>>,
        state: Rc<Cell<bool>>,
    }

    impl Drop for StateDrop {
        fn drop(&mut self) {
            assert!(self.future.get(), "execution must retire before state");
            self.state.set(true);
        }
    }

    #[test]
    fn fixture_cleanup_retires_borrowed_work_before_state_even_with_an_outliving_handle() {
        let future = Rc::new(Cell::new(false));
        let state = Rc::new(Cell::new(false));
        let task = with_application(
            StateDrop {
                future: future.clone(),
                state: state.clone(),
            },
            |app| {
                let access = app.access();
                let guard = FutureDrop(app.read(|state| state.future.clone()));
                let task = access
                    .clone()
                    .spawn(move |_| async move {
                        let _guard = guard;
                        std::future::pending::<()>().await;
                        access.update(|_| panic!("retired continuation resumed"));
                    })
                    .unwrap();
                app.pump();
                task
            },
        );
        assert!(future.get() && state.get());
        drop(task);
    }

    #[test]
    fn queued_wakes_cannot_bypass_the_wait_deadline() {
        with_application((), |app| {
            let task = app
                .access()
                .spawn(|_| {
                    std::future::poll_fn(|cx| {
                        cx.waker().wake_by_ref();
                        std::task::Poll::<()>::Pending
                    })
                })
                .unwrap();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app.run_until(Duration::ZERO, |_| false);
            }));
            assert!(result.is_err());
            drop(task);
        });
    }

    #[test]
    fn the_same_fixture_runs_foreign_completion_without_painting_or_retained_borrows() {
        with_application(0, |app| {
            let (send, result) = futures_channel::oneshot::channel();
            let access = app.access();
            let local = Rc::new(Cell::new(false));
            let completed = local.clone();
            let task = access
                .clone()
                .spawn(move |_| async move {
                    let value = result.await.unwrap();
                    access.update(|state| *state = value).unwrap();
                    completed.set(true);
                })
                .unwrap();
            app.pump();
            app.update(|state| *state = 1);
            std::thread::spawn(move || send.send(42).unwrap())
                .join()
                .unwrap();
            app.run_until(Duration::from_secs(5), |state| *state == 42);
            assert!(local.get());
            drop(task);
        });
    }
}
