//! Async acquisition of immutable camera sources, with optional document-scoped reuse.
use crate::{
    Error,
    scan::DevelopedSource,
    source_loader::Priority,
    source_store::{SourceKey, SourceStore},
};
use std::{future::Future, sync::Arc};

#[cfg(test)]
#[path = "source_provider_tests.rs"]
mod tests;

/// Acquires an owning source lease. Dropping the future retires that request;
/// a native call already in progress keeps its inputs until it finishes.
pub trait SourceProvider: Send + Sync {
    fn load_source(
        &self,
        key: SourceKey,
        priority: Priority,
    ) -> impl Future<Output = Result<Arc<DevelopedSource>, Error>> + Send;
}

/// Reuses the existing store's live leases and optional retained ownership.
/// Store closure ends cache reuse/publication, not the underlying load's authority.
pub struct SourceCache<P> {
    inner: P,
    store: Arc<SourceStore>,
    parent: Option<Arc<SourceStore>>,
}

impl<P> SourceCache<P> {
    pub fn new(inner: P, store: Arc<SourceStore>) -> Self {
        Self {
            inner,
            store,
            parent: None,
        }
    }

    pub fn with_parent(mut self, parent: Option<Arc<SourceStore>>) -> Self {
        self.parent = parent;
        self
    }
}

impl<P: SourceProvider> SourceProvider for SourceCache<P> {
    async fn load_source(
        &self,
        key: SourceKey,
        priority: Priority,
    ) -> Result<Arc<DevelopedSource>, Error> {
        if let Some(source) = self.store.get(&key) {
            return Ok(source);
        }
        let source = match self.parent.as_ref().and_then(|parent| parent.get(&key)) {
            Some(source) => source,
            None => self.inner.load_source(key.clone(), priority).await?,
        };
        Ok(self
            .store
            .publish_source(key, source.clone())?
            .unwrap_or(source))
    }
}
