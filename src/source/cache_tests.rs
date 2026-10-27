use super::*;
use crate::source::{self as source_loading, Priority, SourceLoader, SourceProvider};
use crate::{project::SourceDevelopment, raw::fixture, scan::Scan, source::Charges};
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
    priorities: std::sync::Mutex<Vec<Priority>>,
}

impl Existing {
    fn new(source: &Arc<DevelopedSource>) -> Self {
        Self {
            source: Arc::downgrade(source),
            calls: AtomicUsize::new(0),
            priorities: Default::default(),
        }
    }
}

impl SourceProvider for Existing {
    async fn load_source(
        &self,
        _: SourceKey,
        priority: Priority,
    ) -> Result<Arc<DevelopedSource>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.priorities.lock().unwrap().push(priority);
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
    let store = Arc::new(SourceCache::new(0));
    let cache = store.clone();
    let provider = Existing::new(&original);
    futures_lite::future::block_on(async {
        let first = cache
            .load_source(&provider, key.clone(), source_loading::Priority::Eager)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &original));
        drop(original);
        let second = cache
            .load_source(&provider, key.clone(), source_loading::Priority::Foreground)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
        assert_eq!(*provider.priorities.lock().unwrap(), [Priority::Eager]);
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
    let store = Arc::new(SourceCache::new(bytes));
    let cache = store.clone();
    let provider = Existing::new(&original);
    futures_lite::future::block_on(async {
        let lease = cache
            .load_source(&provider, key.clone(), source_loading::Priority::Foreground)
            .await
            .unwrap();
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
        let lease = cache
            .load_source(&provider, key, source_loading::Priority::Foreground)
            .await
            .unwrap();
        assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
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

fn closed_cache_is_not_authority_to_cancel_an_independent_load() {
    let (_file, key, source) = input();
    let store = Arc::new(SourceCache::new(0));
    store.close();
    let cache = store.clone();
    let provider = Existing::new(&source);
    let lease = futures_lite::future::block_on(cache.load_source(
        &provider,
        key,
        source_loading::Priority::Foreground,
    ))
    .unwrap();
    assert!(Arc::ptr_eq(&lease, &source));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.charges(), Charges::default());
}

#[test]
fn unpolled_acquisition_does_not_start_loading() {
    let (_file, key, source) = input();
    let cache = SourceCache::new(0);
    let provider = Existing::new(&source);
    drop(cache.load_source(&provider, key, source_loading::Priority::Foreground));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn concurrent_document_cache_misses_join_native_work_and_one_waiter_can_leave() {
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
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        if matches!(phase, source_loading::NativePhase::DevelopBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    let loader = scheduler.clone();
    let first_cache = SourceCache::new(0);
    let second_cache = SourceCache::new(0);
    let first_loader = loader.clone();
    let executor = async_executor::Executor::new();
    futures_lite::future::block_on(executor.run(async {
        let first_key = key.clone();
        let first = executor.spawn(async move {
            first_cache
                .load_source(
                    &first_loader,
                    first_key,
                    source_loading::Priority::Foreground,
                )
                .await
        });
        blocking::unblock(move || starts.recv_timeout(Duration::from_secs(5)).unwrap()).await;
        let second = executor.spawn(async move {
            second_cache
                .load_source(&loader, key, source_loading::Priority::Foreground)
                .await
        });
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
fn completed_native_result_is_reused_before_any_cache_wrapper_has_published_it() {
    use std::time::{Duration, Instant};
    let (_file, key, _original) = input();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::DevelopBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    let first = scheduler
        .submit_key(key.clone(), source_loading::Priority::Foreground)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let produced = loop {
        if let Some(source) = first.result().unwrap() {
            break source;
        }
        assert!(Instant::now() < deadline, "native source did not complete");
        std::thread::sleep(Duration::from_millis(1));
    };
    let weak = Arc::downgrade(&produced);
    let store = Arc::new(SourceCache::new(0));
    let provider = scheduler;
    let cache = store;
    let lease = futures_lite::future::block_on(cache.load_source(
        &provider,
        key,
        source_loading::Priority::Foreground,
    ))
    .unwrap();
    assert!(Arc::ptr_eq(&lease, &produced));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "publication handoff must not cause another development"
    );
    drop((first, produced, lease));
    assert!(
        weak.upgrade().is_none(),
        "loader discovery must not become optional strong retention"
    );
}

#[test]
fn native_loader_returns_the_requested_development_and_rejects_a_different_basis() {
    let (_file, key, source) = input();
    let loader = SourceLoader::new();
    let lease = futures_lite::future::block_on(
        loader.load_source(key.clone(), source_loading::Priority::Foreground),
    )
    .unwrap();
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
        futures_lite::future::block_on(
            loader.load_source(wrong, source_loading::Priority::Foreground)
        ),
        Err(Error::Project(_))
    ));
}
