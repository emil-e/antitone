//! Synchronous work on the shared blocking pool, returning worker panics as errors.
use crate::cancellation::Cancellation;

/// Dropping the wait signals cooperative cancellation; a running job keeps its
/// captured inputs and must stop at its own safe checkpoints.
pub(crate) async fn run<T, F>(job: F) -> std::thread::Result<T>
where
    T: Send + 'static,
    F: FnOnce(Cancellation) -> T + Send + 'static,
{
    let cancelled = Cancellation::default();
    let _retire = cancelled.cancel_on_drop();
    #[cfg(test)]
    if let Some(pause) = testing::current() {
        pause.wait().await;
    }
    blocking::unblock(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(cancelled)))
    })
    .await
}

#[cfg(test)]
pub(crate) mod testing {
    use std::{
        cell::RefCell,
        marker::PhantomData,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    thread_local! {
        static PAUSE: RefCell<Option<Arc<Pause>>> = const { RefCell::new(None) };
    }

    pub(super) struct Pause {
        released: AtomicBool,
        changed: event_listener::Event,
    }

    impl Pause {
        pub(super) async fn wait(&self) {
            let changed = self.changed.listen();
            if !self.released.load(Ordering::Acquire) {
                changed.await;
            }
        }
    }

    pub(super) fn current() -> Option<Arc<Pause>> {
        PAUSE.with(|slot| slot.borrow().clone())
    }

    pub(crate) struct PausedWork(Arc<Pause>, PhantomData<Rc<()>>);

    /// Defer dispatch from this test thread. Release wakes all queued jobs concurrently.
    pub(crate) fn pause() -> PausedWork {
        let pause = Arc::new(Pause {
            released: AtomicBool::new(false),
            changed: event_listener::Event::new(),
        });
        PAUSE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "background work is already paused in this test"
            );
            *slot.borrow_mut() = Some(pause.clone());
        });
        PausedWork(pause, PhantomData)
    }

    impl Drop for PausedWork {
        fn drop(&mut self) {
            PAUSE.with(|slot| *slot.borrow_mut() = None);
            self.0.released.store(true, Ordering::Release);
            self.0.changed.notify(usize::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn a_slow_worker_does_not_serialize_unrelated_work() {
        futures_lite::future::block_on(async {
            let (entered, starts) = mpsc::channel();
            let (release, resume) = mpsc::channel();
            let mut slow = Box::pin(run(move |_| {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                1
            }));
            assert!(futures_lite::future::poll_once(&mut slow).await.is_none());
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(run(|_| 2).await.unwrap(), 2);
            release.send(()).unwrap();
            assert_eq!(slow.await.unwrap(), 1);
        });
    }

    #[test]
    fn dropping_before_dispatch_releases_inputs_without_running_the_job() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct Input(Arc<AtomicBool>);
        impl Drop for Input {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        for polled in [false, true] {
            let started = Arc::new(AtomicBool::new(false));
            let ran = started.clone();
            let dropped = Arc::new(AtomicBool::new(false));
            let input = Input(dropped.clone());
            let paused = testing::pause();
            let mut work = Box::pin(run(move |_| {
                let _input = input;
                ran.store(true, Ordering::Relaxed);
            }));
            if polled {
                assert!(
                    futures_lite::future::block_on(futures_lite::future::poll_once(&mut work))
                        .is_none()
                );
            }
            drop(work);
            assert!(dropped.load(Ordering::Relaxed));
            assert!(!started.load(Ordering::Relaxed));
            drop(paused);
        }
    }

    #[test]
    fn dropping_a_running_wait_signals_stop_but_keeps_physical_inputs_until_cleanup() {
        futures_lite::future::block_on(async {
            let input = std::sync::Arc::new(vec![1, 2, 3]);
            let retained = std::sync::Arc::downgrade(&input);
            let (entered, started) = mpsc::channel();
            let (release, resume) = mpsc::channel();
            let (finished, cleaned) = mpsc::channel();
            let mut work = Box::pin(run(move |cancelled| {
                entered.send(cancelled.clone()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(cancelled.is_cancelled());
                let count = input.len();
                drop(input);
                finished.send(count).unwrap();
            }));
            assert!(futures_lite::future::poll_once(&mut work).await.is_none());
            let cancelled = started.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(!cancelled.is_cancelled());
            drop(work);
            assert!(cancelled.is_cancelled());
            assert!(retained.upgrade().is_some());
            release.send(()).unwrap();
            assert_eq!(cleaned.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
            assert!(retained.upgrade().is_none());
        });
    }

    #[test]
    fn dropping_one_wait_does_not_cancel_another_running_job() {
        fn parked() -> (
            impl std::future::Future<Output = std::thread::Result<bool>>,
            mpsc::Receiver<Cancellation>,
            mpsc::Sender<()>,
            mpsc::Receiver<()>,
        ) {
            let (entered, started) = mpsc::channel();
            let (release, resume) = mpsc::channel();
            let (finished, cleaned) = mpsc::channel();
            let work = run(move |cancelled| {
                entered.send(cancelled.clone()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                let stopped = cancelled.is_cancelled();
                finished.send(()).unwrap();
                stopped
            });
            (work, started, release, cleaned)
        }
        futures_lite::future::block_on(async {
            let (first, first_started, release_first, first_done) = parked();
            let (second, second_started, release_second, second_done) = parked();
            let mut first = Box::pin(first);
            let mut second = Box::pin(second);
            assert!(futures_lite::future::poll_once(&mut first).await.is_none());
            assert!(futures_lite::future::poll_once(&mut second).await.is_none());
            let first_signal = first_started.recv_timeout(Duration::from_secs(5)).unwrap();
            let second_signal = second_started.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(first);
            assert!(first_signal.is_cancelled());
            assert!(!second_signal.is_cancelled());
            release_first.send(()).unwrap();
            release_second.send(()).unwrap();
            assert!(!second.await.unwrap());
            second_done.recv_timeout(Duration::from_secs(5)).unwrap();
            first_done.recv_timeout(Duration::from_secs(5)).unwrap();
        });
    }

    #[test]
    fn worker_panics_return_as_errors_and_never_move_execution_onto_the_caller() {
        let caller = std::thread::current().id();
        futures_lite::future::block_on(async {
            assert!(run(|_| panic!("failed computation")).await.is_err());
            assert_ne!(run(|_| std::thread::current().id()).await.unwrap(), caller);
        });
    }
}
