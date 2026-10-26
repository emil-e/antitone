//! Synchronous work on the shared blocking pool, returning worker panics as errors.

pub(crate) async fn run<T, F>(job: F) -> std::thread::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    #[cfg(test)]
    if let Some(pause) = testing::current() {
        pause.wait().await;
    }
    blocking::unblock(move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))).await
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
            let mut slow = Box::pin(run(move || {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                1
            }));
            assert!(futures_lite::future::poll_once(&mut slow).await.is_none());
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(run(|| 2).await.unwrap(), 2);
            release.send(()).unwrap();
            assert_eq!(slow.await.unwrap(), 1);
        });
    }

    #[test]
    fn worker_panics_return_as_errors_and_never_move_execution_onto_the_caller() {
        let caller = std::thread::current().id();
        futures_lite::future::block_on(async {
            assert!(run(|| panic!("failed computation")).await.is_err());
            assert_ne!(run(|| std::thread::current().id()).await.unwrap(), caller);
        });
    }
}
