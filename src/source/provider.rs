use super::{
    Acquisition, PreviewFuture, PreviewLoader, PreviewResult, Priority, SourceCache, SourceFuture,
    SourceKey, SourceProvider,
};
use crate::{Error, scan::CameraPixels};
use std::sync::Arc;

#[cfg(test)]
mod tests;

struct CachedProvider {
    cache: Arc<SourceCache>,
    backend: Arc<dyn SourceProvider>,
}

impl super::SourceLoader {
    pub(crate) fn provider(&self, cache: Option<Arc<SourceCache>>) -> Arc<dyn SourceProvider> {
        let backend: Arc<dyn SourceProvider> = Arc::new(self.clone());
        match cache {
            Some(cache) => cache.provider(backend),
            None => backend,
        }
    }
}

impl SourceCache {
    /// Bind acquisition to the retention owner. Physical jobs receive only captured inputs.
    pub(crate) fn provider(
        self: &Arc<Self>,
        backend: Arc<dyn SourceProvider>,
    ) -> Arc<dyn SourceProvider> {
        Arc::new(CachedProvider {
            cache: self.clone(),
            backend,
        })
    }
}

impl SourceProvider for CachedProvider {
    fn load_source(&self, key: SourceKey, priority: Priority) -> SourceFuture<'_> {
        Box::pin(async move {
            if let Some(source) = self.cache.get(&key) {
                return Ok(source);
            }
            let source = self.backend.load_source(key.clone(), priority).await?;
            if !key.matches(&source) {
                return Err(Error::Project(
                    "developed source differs from requested development".into(),
                ));
            }
            Ok(self
                .cache
                .publish_source(key, source.clone())?
                .unwrap_or(source))
        })
    }

    fn available_proxy(&self, key: &SourceKey) -> Option<Arc<CameraPixels>> {
        self.cache.get_proxy(key)
    }

    fn load_proxy(&self, key: SourceKey, priority: Priority) -> Acquisition<'_, Arc<CameraPixels>> {
        Box::pin(proxy(self, Some(&self.cache), key, priority))
    }

    fn load_preview(&self, key: SourceKey) -> PreviewFuture<'_> {
        Box::pin(async move {
            if let Some(preview) = self.cache.get_preview(key.file()) {
                return Ok(Some(preview));
            }
            let file = key.file().clone();
            let result = self.backend.load_preview(key).await;
            retain_preview(&self.cache, file, &result);
            result
        })
    }
}

pub(super) async fn preview(loader: &PreviewLoader, key: SourceKey) -> PreviewResult {
    let file = key.file().clone();
    let development = key.development().clone();
    let path = key.path().to_owned();
    let read = move || crate::embedded_preview::read(&path, &development);
    loader.load(file, read).await
}

fn retain_preview(cache: &SourceCache, file: super::FileKey, result: &PreviewResult) {
    if let Ok(Some(preview)) = result
        && let Err(error) = cache.publish_preview(file, preview.clone())
    {
        log::warn!(error = error.to_string().as_str(); "preview_cache_publication_failed");
    }
}

pub(super) async fn proxy<P: SourceProvider + ?Sized>(
    provider: &P,
    cache: Option<&SourceCache>,
    key: SourceKey,
    priority: Priority,
) -> Result<Arc<CameraPixels>, Error> {
    if let Some(proxy) = cache.and_then(|cache| cache.get_proxy(&key)) {
        return Ok(proxy);
    }
    let source = provider.load_source(key.clone(), priority).await?;
    if !key.matches(&source) {
        return Err(Error::Project(
            "proxy source differs from requested development".into(),
        ));
    }
    let proxy = crate::background::run(move || {
        CameraPixels::from_scan(&crate::scan::Scan::from_source(source)).map(Arc::new)
    })
    .await
    .map_err(|_| Error::UnusableSample("proxy preparation panicked"))??;
    if let Some(cache) = cache {
        cache.publish_proxy(key, proxy.clone())?;
    }
    Ok(proxy)
}
