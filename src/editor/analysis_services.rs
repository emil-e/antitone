use super::{Document, Error, Scan, SourceId, analysis, source, source_loading};
use crate::source::SourceCache;
use std::sync::Arc;

pub(super) use crate::cancellation::Cancellation;

fn source_input(
    document: &Document,
    id: SourceId,
    cache: Arc<SourceCache>,
) -> Result<source::Request, Error> {
    let project = &document.project;
    let expected = project.source(id)?;
    let path = document.source_path(id)?;
    let wb = expected.development.decode.fixed_wb;
    let file = document.file_key(&path);
    Ok(source::Request {
        reference: Arc::new(
            document
                .reference_exposure(expected)
                .cloned()
                .map_or_else(std::sync::OnceLock::new, std::sync::OnceLock::from),
        ),
        input: project.input,
        path,
        wb: Some(wb),
        expected: Some(expected.clone()),
        cache: Some(cache),
        file,
        correction: None,
    })
}

#[derive(Clone)]
pub(super) struct Services {
    pub developments: source_loading::SourceLoader,
    pub device: wgpu::Device,
    pub cache: Arc<SourceCache>,
    pub exposures: Arc<crate::exposure_metadata::Store>,
}

impl Services {
    pub fn new(
        source: &source::Services,
        cache: Arc<SourceCache>,
        exposures: Arc<crate::exposure_metadata::Store>,
    ) -> Self {
        Self {
            developments: source.developments.clone(),
            device: source.device.clone(),
            cache,
            exposures,
        }
    }

    pub async fn cpu<T: Send + 'static>(
        &self,
        cancelled: &Cancellation,
        job: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<Option<T>, Error> {
        let stopped = cancelled.clone();
        let result = crate::background::run(move || {
            if stopped.is_cancelled() {
                return Ok(None);
            }
            job().map(Some)
        })
        .await;
        let result = result.map_err(|_| Error::UnusableSample("analysis worker panicked"))?;
        if cancelled.is_cancelled() {
            Ok(None)
        } else {
            result
        }
    }

    /// Await a source lease without occupying a blocking-pool thread. Optional
    /// retention belongs to the cache; analysis owns each frame's sampling inputs.
    pub async fn acquire(
        &self,
        request: Arc<analysis::Request>,
        index: usize,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        let cache = self.cache.clone();
        self.acquire_captured(
            move || {
                source_input(
                    &request.document,
                    request.document.project.order[index],
                    cache,
                )
            },
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
        self.acquire_captured(
            move || source_input(&document, source, self.cache.clone()),
            cancelled,
        )
        .await
    }

    async fn acquire_captured(
        &self,
        capture: impl FnOnce() -> Result<source::Request, Error>,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        let mut request = capture()?;
        if let Some(expected) = &request.expected {
            request.reference = self.exposures.memo(expected.id, &request.file.path);
        }
        let key = request.load_key()?;
        let cache = request.cache.clone().expect("analysis document cache");
        let input = request.input;
        let reference = request.reference;

        let loader = self.developments.clone();

        let load = async {
            let source = cache
                .load_source(&loader, key, source_loading::Priority::Foreground)
                .await?;
            crate::background::run(move || {
                crate::photo_preparation::native(Some(&cache), &reference, input, &source)
                    .map(|prepared| Some(prepared.scan()))
            })
            .await
            .map_err(|_| Error::UnusableSample("analysis preparation panicked"))?
        };
        let stop = async {
            cancelled.wait().await;
            Ok(None)
        };
        let result = futures_lite::future::or(stop, load).await;
        if cancelled.is_cancelled() {
            Ok(None)
        } else {
            result
        }
    }
}
