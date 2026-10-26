//! Async acquisition of immutable camera sources, with optional document-scoped reuse.
use crate::{
    Error,
    editor::development,
    scan::DevelopedSource,
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
    ) -> impl Future<Output = Result<Arc<DevelopedSource>, Error>> + Send;
}

/// Native execution without document cache ownership or retention decisions.
#[derive(Clone)]
pub struct SourceLoader {
    developments: development::Scheduler,
}

impl Default for SourceLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceLoader {
    pub fn new() -> Self {
        Self {
            developments: development::Scheduler::new(),
        }
    }
}

impl SourceProvider for SourceLoader {
    async fn load_source(&self, key: SourceKey) -> Result<Arc<DevelopedSource>, Error> {
        let developments = self.developments.clone();
        let request = key.clone();
        let ticket = blocking::unblock(move || developments.submit_key(request)).await?;
        let source = ticket.await?;
        if !key.matches(&source) {
            return Err(Error::Project(
                "developed source differs from requested development".into(),
            ));
        }
        Ok(source)
    }
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
    async fn load_source(&self, key: SourceKey) -> Result<Arc<DevelopedSource>, Error> {
        let store = self.store.clone();
        let parent = self.parent.clone();
        let lookup = key.clone();
        let existing = blocking::unblock(move || {
            store
                .get(&lookup)
                .or_else(|| parent.and_then(|parent| parent.get(&lookup)))
        })
        .await;
        let source = match existing {
            Some(source) => source,
            None => self.inner.load_source(key.clone()).await?,
        };
        let store = self.store.clone();
        blocking::unblock(move || Ok(store.publish_source(key, source.clone())?.unwrap_or(source)))
            .await
    }
}
