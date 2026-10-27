//! Source-provider reuse over the shared pixel storage.
use super::{Priority, SourceKey, SourceProvider, SourceStore};
use crate::{Error, scan::DevelopedSource};
use std::sync::Arc;

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

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
