use super::*;
use crate::{project::SourceDevelopment, raw::fixture, scan::Scan, source_store::Charges};
use std::sync::{
    Weak,
    atomic::{AtomicUsize, Ordering},
};

fn input() -> (
    crate::fixture_path::FixturePath,
    SourceKey,
    Arc<DevelopedSource>,
) {
    let file = fixture::bayer_dng();
    let source = Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone();
    let metadata = source.metadata();
    let image = source.camera_image().unwrap();
    let area = source.area();
    let development = SourceDevelopment {
        decode: metadata.development(source.fixed_wb()),
        lens_correction: metadata.lens_identity().unwrap(),
        dng_development: metadata.dng_identity(),
        dimensions: [image.width, image.height],
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: source.camera_model().to_owned(),
        camera_to_ap1: source.camera_to_ap1().to_cols_array_2d(),
        input_floor: source.input_floor(),
    };
    let key = SourceKey::new(file.path(), &development);
    (file, key, source)
}

struct Existing {
    source: Weak<DevelopedSource>,
    calls: AtomicUsize,
}

impl Existing {
    fn new(source: &Arc<DevelopedSource>) -> Self {
        Self {
            source: Arc::downgrade(source),
            calls: AtomicUsize::new(0),
        }
    }
}

impl SourceProvider for Existing {
    async fn load_source(&self, _: SourceKey) -> Result<Arc<DevelopedSource>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.source
            .upgrade()
            .ok_or(Error::UnusableSample("fixture lease expired"))
    }
}

#[test]
fn zero_retention_reuses_live_consumer_leases_without_owning_the_pixels() {
    let (_file, key, original) = input();
    let bytes = original.retained_bytes().unwrap();
    let weak = Arc::downgrade(&original);
    let store = Arc::new(SourceStore::new(0));
    let cache = SourceCache::new(Existing::new(&original), store.clone());
    futures_lite::future::block_on(async {
        let first = cache.load_source(key.clone()).await.unwrap();
        assert!(Arc::ptr_eq(&first, &original));
        drop(original);
        let second = cache.load_source(key.clone()).await.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.inner.calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            store.charges(),
            Charges {
                retained: 0,
                leased_only: bytes
            }
        );
        drop(first);
        assert!(weak.upgrade().is_some());
        drop(second);
    });
    assert!(weak.upgrade().is_none());
    assert_eq!(store.charges(), Charges::default());
}

#[test]
fn cache_budget_owns_a_keepalive_after_consumers_finish() {
    let (_file, key, original) = input();
    let bytes = original.retained_bytes().unwrap();
    let weak = Arc::downgrade(&original);
    let store = Arc::new(SourceStore::new(bytes));
    let cache = SourceCache::new(Existing::new(&original), store.clone());
    futures_lite::future::block_on(async {
        let lease = cache.load_source(key.clone()).await.unwrap();
        drop(original);
        drop(lease);
        assert!(weak.upgrade().is_some());
        assert_eq!(
            store.charges(),
            Charges {
                retained: bytes,
                leased_only: 0
            }
        );
        let lease = cache.load_source(key).await.unwrap();
        assert_eq!(cache.inner.calls.load(Ordering::Relaxed), 1);
        store.set_budget(0);
        assert_eq!(
            store.charges(),
            Charges {
                retained: 0,
                leased_only: bytes
            }
        );
        drop(lease);
    });
    assert!(weak.upgrade().is_none());
}

#[test]
fn candidate_reuse_registers_a_lease_without_taking_parent_cache_ownership() {
    let (_file, key, source) = input();
    let bytes = source.retained_bytes().unwrap();
    let parent = Arc::new(SourceStore::new(bytes));
    parent
        .publish_source(key.clone(), source.clone())
        .unwrap()
        .unwrap();
    let candidate = Arc::new(SourceStore::new(0));
    let cache = SourceCache::new(Existing::new(&source), candidate.clone())
        .with_parent(Some(parent.clone()));
    let lease = futures_lite::future::block_on(cache.load_source(key)).unwrap();
    assert!(Arc::ptr_eq(&source, &lease));
    assert_eq!(cache.inner.calls.load(Ordering::Relaxed), 0);
    assert_eq!(parent.charges().retained, bytes);
    assert_eq!(
        candidate.charges(),
        Charges {
            retained: 0,
            leased_only: bytes
        }
    );
    candidate.close();
    assert_eq!(parent.charges().retained, bytes);
    assert!(lease.camera_image().is_ok());
}

#[test]
fn closed_cache_is_not_authority_to_cancel_an_independent_load() {
    let (_file, key, source) = input();
    let store = Arc::new(SourceStore::new(0));
    store.close();
    let cache = SourceCache::new(Existing::new(&source), store.clone());
    let lease = futures_lite::future::block_on(cache.load_source(key)).unwrap();
    assert!(Arc::ptr_eq(&lease, &source));
    assert_eq!(cache.inner.calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.charges(), Charges::default());
}

#[test]
fn unpolled_acquisition_does_not_start_loading() {
    let (_file, key, source) = input();
    let cache = SourceCache::new(Existing::new(&source), Arc::new(SourceStore::new(0)));
    drop(cache.load_source(key));
    assert_eq!(cache.inner.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn concurrent_cache_misses_join_native_work_and_one_waiter_can_leave() {
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };
    let (_file, key, _original) = input();
    let (entered, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let developments = Arc::new(AtomicUsize::new(0));
    let observed = developments.clone();
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        if matches!(phase, development::NativePhase::DevelopBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    let loader = SourceLoader {
        developments: scheduler.clone(),
    };
    let cache = Arc::new(SourceCache::new(loader, Arc::new(SourceStore::new(0))));
    let executor = async_executor::Executor::new();
    futures_lite::future::block_on(executor.run(async {
        let first_cache = cache.clone();
        let first_key = key.clone();
        let first = executor.spawn(async move { first_cache.load_source(first_key).await });
        blocking::unblock(move || starts.recv_timeout(Duration::from_secs(5)).unwrap()).await;
        let second_cache = cache.clone();
        let second = executor.spawn(async move { second_cache.load_source(key).await });
        let deadline = Instant::now() + Duration::from_secs(5);
        while scheduler.waiter_count() != 2 {
            assert!(Instant::now() < deadline, "matching requests did not join");
            futures_lite::future::yield_now().await;
        }
        drop(first);
        while scheduler.waiter_count() != 1 {
            assert!(
                Instant::now() < deadline,
                "departed waiter still owns its demand"
            );
            futures_lite::future::yield_now().await;
        }
        release.send(()).unwrap();
        let lease = second.await.unwrap();
        assert_eq!(developments.load(Ordering::Relaxed), 1);
        assert!(lease.camera_image().is_ok());
    }));
}

#[test]
fn native_loader_returns_the_requested_development_and_rejects_a_different_basis() {
    let (_file, key, source) = input();
    let loader = SourceLoader::new();
    let lease = futures_lite::future::block_on(loader.load_source(key.clone())).unwrap();
    assert!(key.matches(&lease));
    assert_eq!(
        lease.camera_image().unwrap().pixel_samples(16, 16).unwrap(),
        source
            .camera_image()
            .unwrap()
            .pixel_samples(16, 16)
            .unwrap()
    );
    let mut different = key.development().clone();
    different.camera_model = "wrong source basis".into();
    let wrong = SourceKey::new(key.path(), &different);
    assert!(matches!(
        futures_lite::future::block_on(loader.load_source(wrong)),
        Err(Error::Project(_))
    ));
}
