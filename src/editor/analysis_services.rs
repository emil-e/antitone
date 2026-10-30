use super::{Document, Error, Scan, SourceId, analysis, source, source_loading};
use crate::source::SourceProvider;
use std::sync::Arc;

pub(super) use crate::cancellation::Cancellation;

fn source_input(
    document: &Document,
    id: SourceId,
    sources: Arc<dyn SourceProvider>,
) -> Result<source::Request, Error> {
    let expected = document.project.source(id)?;
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
        input: document.project.input,
        path,
        wb: Some(wb),
        expected: Some(expected.clone()),
        sources: Some(sources),
        file,
        correction: None,
    })
}

#[derive(Clone)]
pub(super) struct Services {
    pub sources: Arc<dyn SourceProvider>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderers: Arc<crate::renderer::RendererCache>,
    pub exposures: Arc<crate::exposure_metadata::Store>,
}

impl Services {
    pub fn new(
        source: &source::Services,
        sources: Arc<dyn SourceProvider>,
        exposures: Arc<crate::exposure_metadata::Store>,
    ) -> Self {
        Self {
            sources,
            device: source.device.clone(),
            queue: source.queue.clone(),
            renderers: source.renderers.clone(),
            exposures,
        }
    }

    pub async fn cpu<T: Send + 'static>(
        &self,
        cancelled: &Cancellation,
        job: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<Option<T>, Error> {
        let stopped = cancelled.clone();
        let result = crate::background::run(move |_| {
            if stopped.is_cancelled() {
                return Ok(None);
            }
            job().map(Some)
        })
        .await
        .map_err(|_| Error::UnusableSample("analysis worker panicked"))?;
        if cancelled.is_cancelled() {
            Ok(None)
        } else {
            result
        }
    }

    pub async fn acquire(
        &self,
        request: Arc<analysis::Request>,
        index: usize,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        self.acquire_captured(
            || {
                source_input(
                    &request.document,
                    request.document.project.order[index],
                    self.sources.clone(),
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
            || source_input(&document, source, self.sources.clone()),
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
        let load = async {
            crate::photo_preparation::acquire(
                self.sources.as_ref(),
                key,
                source_loading::Priority::Foreground,
                request.input,
                request.reference,
            )
            .await
            .map(|prepared| Some(prepared.scan()))
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
