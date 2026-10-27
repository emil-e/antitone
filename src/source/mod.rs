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
pub trait SourceProvider: Send + Sync {
    fn load_source(
        &self,
        key: SourceKey,
        priority: Priority,
    ) -> impl std::future::Future<
        Output = Result<std::sync::Arc<crate::scan::DevelopedSource>, crate::Error>,
    > + Send;
}
