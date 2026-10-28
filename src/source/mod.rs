//! Immutable source acquisition, native execution and optional pixel retention.
mod identity;
#[cfg(feature = "raw")]
pub(crate) use identity::FileKey;
pub use identity::FileRevision;

#[cfg(all(feature = "raw", feature = "project"))]
mod key;
#[cfg(all(feature = "raw", feature = "project"))]
pub use key::SourceKey;

#[cfg(all(feature = "raw", feature = "project"))]
mod cache;
#[cfg(all(feature = "raw", feature = "project"))]
pub use cache::{Charges, SourceCache};

#[cfg(feature = "editor")]
mod preview;
#[cfg(feature = "editor")]
pub(crate) use preview::PreviewLoader;

#[cfg(all(feature = "raw", feature = "project"))]
pub(crate) type PreviewResult =
    Result<Option<std::sync::Arc<crate::embedded_preview::Preview>>, String>;

#[cfg(feature = "editor")]
mod loader;
#[cfg(all(test, feature = "editor"))]
pub(crate) use loader::{NativePhase, Request};
#[cfg(feature = "editor")]
pub use loader::{Priority, SourceLoader};
#[cfg(feature = "editor")]
mod metadata;
#[cfg(feature = "editor")]
pub(crate) use metadata::{
    metadata_development, open_source, source_metadata, verify_scan, verify_source,
};

/// Acquires an owning source lease. Dropping the future retires that request;
/// a native call already in progress keeps its inputs until it finishes.
#[cfg(feature = "editor")]
pub(crate) trait SourceProvider: Send + Sync {
    fn load_source(&self, key: SourceKey, priority: Priority) -> SourceFuture<'_>;

    fn available_proxy(
        &self,
        _key: &SourceKey,
    ) -> Option<std::sync::Arc<crate::scan::CameraPixels>> {
        None
    }

    fn load_proxy(
        &self,
        key: SourceKey,
        priority: Priority,
    ) -> Acquisition<'_, std::sync::Arc<crate::scan::CameraPixels>> {
        Box::pin(provider::proxy(self, None, key, priority))
    }

    fn load_preview(&self, key: SourceKey) -> PreviewFuture<'_> {
        Box::pin(async move { provider::preview(&PreviewLoader::default(), key).await })
    }
}

#[cfg(feature = "editor")]
pub(crate) type Acquisition<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, crate::Error>> + Send + 'a>>;
#[cfg(feature = "editor")]
pub(crate) type SourceFuture<'a> = Acquisition<'a, std::sync::Arc<crate::scan::DevelopedSource>>;
#[cfg(feature = "editor")]
pub(crate) type PreviewFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = PreviewResult> + Send + 'a>>;

#[cfg(feature = "editor")]
mod provider;
