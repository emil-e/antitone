use super::*;
use crate::{
    project::SourceDevelopment,
    scan::{DevelopedSource, Scan},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Backend {
    source: Arc<DevelopedSource>,
    calls: AtomicUsize,
}
impl SourceProvider for Backend {
    fn load_source(&self, _: SourceKey, _: Priority) -> SourceFuture<'_> {
        Box::pin(async {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.source.clone())
        })
    }
}

#[test]
fn scoped_and_cacheless_providers_share_one_acquisition_contract_without_sharing_retention() {
    let file = crate::raw::fixture::bayer_dng();
    let scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
    let development =
        SourceDevelopment::from_metadata(&scan.source().metadata(), scan.source().fixed_wb())
            .unwrap();
    let key = SourceKey::capture(file.path(), &development).unwrap();
    let backend = Arc::new(Backend {
        source: scan.source().clone(),
        calls: AtomicUsize::new(0),
    });
    let cache = Arc::new(SourceCache::new(1 << 20));
    let owner = Arc::downgrade(&cache);
    let scoped: Arc<dyn SourceProvider> = cache.provider(backend.clone());
    let cacheless: Arc<dyn SourceProvider> = backend.clone();
    drop(cache);
    assert!(
        owner.upgrade().is_some(),
        "the scoped provider owns its retention"
    );
    let load = |provider: &dyn SourceProvider| {
        futures_lite::future::block_on(provider.load_source(key.clone(), Priority::Foreground))
    };
    let first = load(scoped.as_ref()).unwrap();
    let reused = load(scoped.as_ref()).unwrap();
    assert!(Arc::ptr_eq(&first, &reused));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    let proxy =
        futures_lite::future::block_on(scoped.load_proxy(key.clone(), Priority::Foreground))
            .unwrap();
    let reused_proxy =
        futures_lite::future::block_on(scoped.load_proxy(key.clone(), Priority::Foreground))
            .unwrap();
    assert!(Arc::ptr_eq(&proxy, &reused_proxy));
    assert_eq!(
        backend.calls.load(Ordering::SeqCst),
        1,
        "proxy preparation reuses acquired native pixels"
    );
    let pixels = Arc::downgrade(&proxy);
    drop(reused_proxy);
    drop(scoped);
    assert!(
        owner.upgrade().is_none(),
        "pixel leases do not retain the cache"
    );
    assert!(!proxy.pixels.is_empty());
    drop(proxy);
    assert!(
        pixels.upgrade().is_none(),
        "only the scoped cache and consumer retained the proxy"
    );
    assert!(Arc::ptr_eq(&load(cacheless.as_ref()).unwrap(), &first));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
}
