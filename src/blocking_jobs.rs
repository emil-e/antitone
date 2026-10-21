//! Admission for synchronous jobs on the established blocking pool.
use async_lock::Semaphore;
use std::{panic::AssertUnwindSafe, sync::Arc};

#[cfg(test)]
#[path = "blocking_jobs_tests.rs"]
mod tests;

pub(crate) type Admission = Arc<async_lock::SemaphoreGuardArc>;

#[derive(Clone)]
pub(crate) struct BlockingJobs(Arc<Semaphore>);

impl BlockingJobs {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self(Arc::new(Semaphore::new(capacity)))
    }

    /// Background GPU admission must also respect physical foreground work after
    /// its UI continuation has retired. This is a nonblocking readiness probe,
    /// not a reservation for a subsequent background job.
    pub fn idle(&self) -> bool {
        self.0.try_acquire().is_some()
    }

    /// Hold physical admission for controlled worker/continuation fixtures.
    #[cfg(test)]
    pub fn hold(&self) -> async_lock::SemaphoreGuardArc {
        self.0.acquire_arc_blocking()
    }

    /// Cancellation drops a queued job. Once running, the closure retains its
    /// permit and captured resources until it returns, independently of its caller.
    pub async fn run<T, F>(&self, job: F) -> std::thread::Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.run_admitted(move |_| job()).await
    }

    /// A native producer can retain admission after this short submission job
    /// returns. Its caller awaits a ticket without occupying a blocking worker.
    pub async fn run_admitted<T, F>(&self, job: F) -> std::thread::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Admission) -> T + Send + 'static,
    {
        let permit = Arc::new(self.0.acquire_arc().await);
        Self::execute(permit, job).await
    }

    /// Stop an unstarted admission wait, but await any admitted job's acknowledgement.
    /// Dropping the caller still retires its continuation without interrupting running work.
    pub async fn run_admitted_cancellable<T, F>(
        &self,
        cancelled: impl std::future::Future<Output = ()>,
        job: F,
    ) -> Option<std::thread::Result<T>>
    where
        T: Send + 'static,
        F: FnOnce(Admission) -> T + Send + 'static,
    {
        let acquire = async { Some(Arc::new(self.0.acquire_arc().await)) };
        let stop = async {
            cancelled.await;
            None
        };
        let permit = futures_lite::future::or(stop, acquire).await?;
        Some(Self::execute(permit, job).await)
    }

    async fn execute<T, F>(permit: Admission, job: F) -> std::thread::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Admission) -> T + Send + 'static,
    {
        blocking::unblock(move || {
            let _physical = permit.clone();
            std::panic::catch_unwind(AssertUnwindSafe(|| job(permit)))
        })
        .await
    }
}
