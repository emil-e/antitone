//! Scoped UI-thread execution with borrowed access to application state.
use async_executor::{LocalExecutor, Task};
use std::{
    cell::RefCell,
    future::{Future, pending},
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context, Wake, Waker},
};

#[cfg(test)]
#[path = "application_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "application_test_support.rs"]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "application_context_tests.rs"]
mod context_tests;

type Repaint = Arc<dyn Fn() + Send + Sync>;

struct HostWake(Box<dyn Fn() + Send + Sync>);

impl Wake for HostWake {
    fn wake(self: Arc<Self>) {
        (self.0)();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        (self.0)();
    }
}

/// The host drives local workflows outside painting. Borrowed inputs and state
/// must outlive this executor; blocking/domain services retain their own policy.
pub struct Application<'scope> {
    runner: Option<Pin<Box<dyn Future<Output = ()> + 'scope>>>,
    executor: Option<Rc<LocalExecutor<'scope>>>,
    waker: Waker,
    repaint: Repaint,
}

impl<'scope> Application<'scope> {
    pub fn new(
        wake: impl Fn() + Send + Sync + 'static,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let executor = Rc::new(LocalExecutor::new());
        let running = executor.clone();
        let mut application = Self {
            runner: Some(Box::pin(async move { running.run(pending()).await })),
            executor: Some(executor),
            waker: Waker::from(Arc::new(HostWake(Box::new(wake)))),
            repaint: Arc::new(repaint),
        };
        // Install the host waker before it can sleep or receive the first task.
        application.drive();
        application
    }

    /// The slot permits deferred native initialization. Initialize it before
    /// driving spawned tasks, and retire execution before taking the state out.
    pub fn access<T: 'scope>(&self, state: &'scope RefCell<Option<T>>) -> UiContext<'scope, T> {
        UiContext {
            state: Some(state),
            executor: self
                .executor
                .as_ref()
                .map(Rc::downgrade)
                .unwrap_or_default(),
            repaint: self.repaint.clone(),
        }
    }

    /// Poll one cooperative executor turn. The waker schedules another host
    /// event when needed; suspended tasks do not require redraws or a timer.
    pub fn drive(&mut self) {
        if let Some(runner) = &mut self.runner {
            let result = runner.as_mut().poll(&mut Context::from_waker(&self.waker));
            debug_assert!(result.is_pending());
        }
    }

    pub fn close(&mut self) {
        // Release the runner's executor reference too, destroying local futures
        // on this thread before the host retires application/window/GPU state.
        self.runner = None;
        self.executor = None;
    }
}

impl Drop for Application<'_> {
    fn drop(&mut self) {
        self.close();
    }
}

/// Borrowed UI-thread state access, task spawning and update notification.
/// Use the existing mutable model during synchronous UI, and short context
/// transactions in continuations; overlapping transactions still panic.
pub struct UiContext<'scope, T> {
    state: Option<&'scope RefCell<Option<T>>>,
    executor: Weak<LocalExecutor<'scope>>,
    repaint: Repaint,
}

impl<'scope, T: 'scope> UiContext<'scope, T> {
    fn state(&self) -> &'scope RefCell<Option<T>> {
        self.state
            .expect("workflow input requires a live application fixture")
    }

    pub fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let state = self.state().borrow();
        read(state.as_ref().expect("application initialized"))
    }

    /// Notify after releasing the borrow, including successful no-op updates.
    /// Reads are silent; this is not field-level change detection.
    pub fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.state();
        let _executor = self.executor.upgrade()?;
        let result = self.with_mut(update);
        (self.repaint)();
        Some(result)
    }

    /// Defer the factory itself until the host polls outside the UI borrow.
    /// Keep the returned library task in its logical owner: dropping it cancels
    /// the continuation, not already-running synchronous work.
    pub fn spawn<R, F, Fut>(&self, body: F) -> Option<Task<R>>
    where
        R: 'scope,
        F: FnOnce(UiContext<'scope, T>) -> Fut + 'scope,
        Fut: Future<Output = R> + 'scope,
    {
        self.state();
        let executor = self.executor.upgrade()?;
        let cx = self.clone();
        Some(executor.spawn(async move { body(cx).await }))
    }

    // Controller/layout tests have no application state or execution. Starting
    // work through this context is a test error, not a fake closed executor.
    #[cfg(test)]
    pub(crate) fn render_only() -> Self {
        Self {
            state: None,
            executor: Weak::new(),
            repaint: Arc::new(|| {}),
        }
    }

    /// Bind another independently owned state cell in the same execution scope.
    pub fn for_state<U: 'scope>(&self, state: &'scope RefCell<Option<U>>) -> UiContext<'scope, U> {
        self.state();
        UiContext {
            state: Some(state),
            executor: self.executor.clone(),
            repaint: self.repaint.clone(),
        }
    }

    // Host callbacks already control repaint delivery. They use the same short
    // borrowing boundary without turning every presentation pass into a redraw.
    pub(crate) fn with_mut<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
        let mut state = self.state().borrow_mut();
        update(state.as_mut().expect("application initialized"))
    }
}

impl<T> Clone for UiContext<'_, T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state,
            executor: self.executor.clone(),
            repaint: self.repaint.clone(),
        }
    }
}
