use super::{Document, Error, Scan, SourceId, analysis, development, source};
use std::sync::Arc;

pub(super) use crate::cancellation::Cancellation;

fn source_input(document: &Document, id: SourceId) -> Result<development::Request, Error> {
    let project = &document.project;
    let expected = project.source(id)?;
    let path = document.source_path(id)?;
    let wb = project
        .development
        .as_ref()
        .ok_or(Error::UnusableSample("no roll development"))?
        .fixed_wb;
    let cache = source::Cache::new(&path, Some(expected), Some(wb), document);
    Ok(development::Request {
        path,
        wb: Some(wb),
        expected: Some(expected.clone()),
        basis: project.camera_basis.clone(),
        file: cache.file,
        key: cache.key,
        store: Some(cache.store),
        parent: cache.parent,
        retain: false,
    })
}

#[derive(Clone)]
pub(super) struct Services {
    pub jobs: crate::blocking_jobs::BlockingJobs,
    pub developments: development::Scheduler,
    pub device: wgpu::Device,
}

impl From<&source::Services> for Services {
    fn from(source: &source::Services) -> Self {
        Self {
            jobs: source.jobs.clone(),
            developments: source.developments.clone(),
            device: source.device.clone(),
        }
    }
}

impl Services {
    pub async fn cpu<T: Send + 'static>(
        &self,
        cancelled: &Cancellation,
        job: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<Option<T>, Error> {
        let stopped = cancelled.clone();
        let result = self
            .jobs
            .run_admitted_cancellable(cancelled.wait(), move |_| {
                if stopped.is_cancelled() {
                    return Ok(None);
                }
                job().map(Some)
            })
            .await;
        let Some(result) = result else {
            return Ok(None);
        };
        let result = result.map_err(|_| Error::UnusableSample("analysis worker panicked"))?;
        if cancelled.is_cancelled() {
            Ok(None)
        } else {
            result
        }
    }

    /// Native production owns admission through cleanup. Awaiting it holds no blocking
    /// worker, and analysis demands do not retain full RAWs as optional cache data.
    pub async fn acquire(
        &self,
        request: Arc<analysis::Request>,
        index: usize,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        self.acquire_captured(
            move || source_input(&request.document, request.document.project.order[index]),
            cancelled,
        )
        .await
    }

    pub async fn acquire_source(
        &self,
        document: Arc<Document>,
        source: SourceId,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        self.acquire_captured(move || source_input(&document, source), cancelled)
            .await
    }

    async fn acquire_captured(
        &self,
        capture: impl FnOnce() -> Result<development::Request, Error> + Send + 'static,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        let scheduler = self.developments.clone();
        let retired = cancelled.clone();
        let result = async {
            let Some(submission) = self
                .jobs
                .run_admitted_cancellable(cancelled.wait(), move |admission| {
                    if retired.is_cancelled() {
                        return Err(Error::UnusableSample("analysis retired"));
                    }
                    scheduler.submit_admitted(
                        capture()?,
                        development::Priority::Foreground,
                        admission,
                    )
                })
                .await
            else {
                return Ok(None);
            };
            let ticket = submission.map_err(|_| {
                Error::UnusableSample("analysis source submission worker panicked")
            })??;
            // Native demand can be abandoned at its service boundary, as with the
            // synchronous ticket wait; the producer still owns mutable RAW cleanup.
            let wait = async { ticket.await.map(Some) };
            let stop = async {
                cancelled.wait().await;
                Ok(None)
            };
            futures_lite::future::or(stop, wait)
                .await
                .map(|source| source.map(Scan::from_source))
        }
        .await;
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        result
    }
}
