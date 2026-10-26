use super::{Document, Error, Scan, SourceId, analysis, development, source};
use crate::source_provider::{SourceLoader, SourceProvider};
use std::sync::Arc;

pub(super) use crate::cancellation::Cancellation;

fn source_input(document: &Document, id: SourceId) -> Result<source::Request, Error> {
    let project = &document.project;
    let expected = project.source(id)?;
    let path = document.source_path(id)?;
    let wb = expected.development.decode.fixed_wb;
    let cache = source::Cache::new(&path, Some(expected), document);
    Ok(source::Request {
        path,
        wb: Some(wb),
        expected: Some(expected.clone()),
        cache: Some(cache),
        correction: None,
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
        capture: impl FnOnce() -> Result<source::Request, Error> + Send + 'static,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        let retired = cancelled.clone();
        let Some(captured) = self
            .jobs
            .run_admitted_cancellable(cancelled.wait(), move |_| {
                if retired.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let request = capture()?;
                let key = request.load_key()?;
                Ok((key, request.cache.expect("analysis document cache")))
            })
            .await
        else {
            return Ok(None);
        };
        let (key, cache) = captured
            .map_err(|_| Error::UnusableSample("analysis source capture worker panicked"))??;
        let store = cache.store.clone();
        let changed = store.listen_for_close();
        let loader = SourceLoader::from_scheduler(
            self.developments.clone(),
            development::Priority::Foreground,
        );
        let provider = cache.provider(loader);
        let load = async {
            provider
                .load_source(key)
                .await
                .map(|source| Some(Scan::from_source(source)))
        };
        let stop = async {
            cancelled.wait().await;
            Ok(None)
        };
        let close = async {
            if !store.is_closed() {
                changed.await;
            }
            Err(Error::UnusableSample("analysis document retired"))
        };
        let result = futures_lite::future::or(stop, futures_lite::future::or(close, load)).await;
        if cancelled.is_cancelled() {
            Ok(None)
        } else {
            result
        }
    }
}
