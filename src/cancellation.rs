//! Cooperative operation cancellation, independent of task and resource ownership.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<Signal>);

#[derive(Default)]
struct Signal {
    requested: Arc<AtomicBool>,
    changed: event_listener::Event,
}

impl Cancellation {
    pub fn cancel(&self) {
        if !self.0.requested.swap(true, Ordering::Relaxed) {
            self.0.changed.notify(usize::MAX);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.requested.load(Ordering::Relaxed)
    }

    pub async fn wait(&self) {
        let changed = self.0.changed.listen();
        if !self.is_cancelled() {
            changed.await;
        }
    }

    /// Poll-only adapter for existing services. Request cancellation through
    /// `cancel`, not this flag, so asynchronous waiters are also woken.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.requested.clone()
    }

    /// Keep this guard in the operation's continuation, including before its first poll.
    pub fn cancel_on_drop(&self) -> CancelOnDrop {
        CancelOnDrop(self.clone())
    }
}

pub(crate) struct CancelOnDrop(Cancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::{block_on, poll_once};

    #[test]
    fn cancellation_wakes_all_waiters_and_remains_observable() {
        block_on(async {
            let signal = Cancellation::default();
            let mut first = Box::pin(signal.wait());
            let mut second = Box::pin(signal.wait());
            assert!(poll_once(&mut first).await.is_none());
            assert!(poll_once(&mut second).await.is_none());
            signal.clone().cancel();
            signal.cancel();
            assert!(signal.is_cancelled());
            assert_eq!(poll_once(&mut first).await, Some(()));
            assert_eq!(poll_once(&mut second).await, Some(()));
            assert_eq!(poll_once(signal.wait()).await, Some(()));
        });
    }

    #[test]
    fn retiring_an_unpolled_continuation_signals_only_its_operation() {
        let signal = Cancellation::default();
        let unrelated = Cancellation::default();
        let guard = signal.cancel_on_drop();
        let continuation = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };
        drop(continuation);
        assert!(signal.is_cancelled());
        assert!(!unrelated.is_cancelled());
    }

    #[test]
    fn cancellation_actually_wakes_a_parked_future() {
        use std::{
            future::Future,
            pin::pin,
            sync::atomic::AtomicUsize,
            task::{Context, Poll, Wake, Waker},
        };
        struct Wakes(AtomicUsize);
        impl Wake for Wakes {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let signal = Cancellation::default();
        let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        let mut wait = pin!(signal.wait());
        assert_eq!(wait.as_mut().poll(&mut cx), Poll::Pending);
        signal.cancel();
        assert!(
            wakes.0.load(Ordering::Relaxed) > 0,
            "polling a flag alone must not masquerade as wake delivery"
        );
        assert_eq!(wait.as_mut().poll(&mut cx), Poll::Ready(()));
    }
}
