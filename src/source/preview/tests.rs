use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn poll<F: Future>(future: Pin<&mut F>, wakes: &Arc<Wakes>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(&Waker::from(wakes.clone())))
}

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "preview boundary never completed"
        );
        std::thread::yield_now();
    }
}

fn image() -> Arc<Preview> {
    Arc::new(Preview::srgb_fixture(
        image::RgbImage::from_pixel(16, 8, image::Rgb([80, 100, 120])),
        0,
    ))
}

#[test]
fn dropping_the_first_preview_waiter_preserves_joined_decode_and_wakes_the_other() {
    let file = crate::fixture_path::FixturePath::write("preview.raw", &[1]);
    let key = FileKey::new(file.path());
    let loader = PreviewLoader::default();
    let loads = Arc::new(AtomicUsize::new(0));
    let wakes = Arc::new(Wakes::default());
    let (entered, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let calls = loads.clone();
    let caller = std::thread::current().id();
    let mut first = Box::pin(loader.load(key.clone(), move || {
        assert_ne!(std::thread::current().id(), caller);
        calls.fetch_add(1, Ordering::SeqCst);
        entered.send(()).unwrap();
        resume.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(Some(image()))
    }));
    assert!(poll(first.as_mut(), &wakes).is_pending());
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut second = Box::pin(loader.load(key.clone(), || panic!("duplicate decoding")));
    assert!(poll(second.as_mut(), &wakes).is_pending());
    let other_wakes = Arc::new(Wakes::default());
    let mut third = Box::pin(loader.load(key.clone(), || panic!("duplicate decoding")));
    assert!(poll(third.as_mut(), &other_wakes).is_pending());
    drop(first);
    wakes.0.store(0, Ordering::SeqCst);
    other_wakes.0.store(0, Ordering::SeqCst);
    release.send(()).unwrap();
    wait(|| wakes.0.load(Ordering::SeqCst) != 0 && other_wakes.0.load(Ordering::SeqCst) != 0);
    let Poll::Ready(Ok(Some(other))) = poll(third.as_mut(), &other_wakes) else {
        panic!("another joined consumer was stranded")
    };
    let Poll::Ready(Ok(Some(preview))) = poll(second.as_mut(), &wakes) else {
        panic!("notified joiner did not receive its preview")
    };
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&preview, &other));
    drop(third);
    assert_eq!(preview.rgb.get_pixel(0, 0).0, [80, 100, 120]);
    drop(second);
    let reused = futures_lite::future::block_on(
        loader.load(key, || panic!("live pixels must remain discoverable")),
    )
    .unwrap()
    .unwrap();
    assert!(Arc::ptr_eq(&preview, &reused));
}

#[test]
fn preview_discovery_does_not_retain_completed_pixels_outside_the_lru() {
    let file = crate::fixture_path::FixturePath::write("preview.raw", &[1]);
    let key = FileKey::new(file.path());
    let cache = Arc::new(super::super::SourceCache::new(1 << 20));
    let loader = PreviewLoader::default();
    let preview =
        futures_lite::future::block_on(cache.preview(&loader, key.clone(), || Ok(Some(image()))))
            .unwrap()
            .unwrap();
    let weak = Arc::downgrade(&preview);
    drop(preview);
    let reused = futures_lite::future::block_on(cache.preview(&loader, key.clone(), || {
        panic!("retained preview decoded twice")
    }))
    .unwrap()
    .unwrap();
    assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), &reused));
    cache.set_budget(0);
    drop(reused);
    wait(|| weak.upgrade().is_none());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    futures_lite::future::block_on(cache.preview(&loader, key, move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Some(image()))
    }))
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_new_document_retains_live_preview_pixels_without_redecoding() {
    let file = crate::fixture_path::FixturePath::write("preview.raw", &[1]);
    let key = FileKey::new(file.path());
    let loader = PreviewLoader::default();
    let preview = futures_lite::future::block_on(loader.load(key.clone(), || Ok(Some(image()))))
        .unwrap()
        .unwrap();
    let cache = Arc::new(super::super::SourceCache::new(1 << 20));
    let joined = futures_lite::future::block_on(
        cache.preview(&loader, key.clone(), || panic!("live preview redeveloped")),
    )
    .unwrap()
    .unwrap();
    assert!(Arc::ptr_eq(&preview, &joined));
    let weak = Arc::downgrade(&preview);
    drop((preview, joined));
    let retained = cache
        .get_preview(&key)
        .expect("the new document must own optional retention");
    assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), &retained));
}

#[test]
fn preview_decode_panic_completes_its_waiter_as_an_error() {
    let file = crate::fixture_path::FixturePath::write("preview.raw", &[1]);
    let loader = PreviewLoader::default();
    let wakes = Arc::new(Wakes::default());
    let mut load = Box::pin(loader.load(FileKey::new(file.path()), || panic!("decode failure")));
    match poll(load.as_mut(), &wakes) {
        Poll::Ready(Err(_)) => {}
        Poll::Pending => {
            wait(|| wakes.0.load(Ordering::SeqCst) != 0);
            assert!(matches!(poll(load.as_mut(), &wakes), Poll::Ready(Err(_))));
        }
        Poll::Ready(Ok(_)) => panic!("a decoder panic must be reported as failure"),
    }
}
