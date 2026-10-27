use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn image() -> Arc<Preview> {
    Arc::new(Preview {
        rgb: image::RgbImage::from_pixel(16, 8, image::Rgb([80, 100, 120])),
        capture_flip: 0,
    })
}

fn file() -> crate::fixture_path::FixturePath {
    crate::fixture_path::FixturePath::write("source.raw", &[1])
}

#[test]
fn overlapping_preview_consumers_share_extraction_across_forks_without_optional_retention() {
    for budget in [0, 1 << 20] {
        let path = file();
        let key = FileKey::new(path.path());
        let store = Arc::new(SourceStore::new(budget));
        let loads = Arc::new(AtomicUsize::new(0));
        let (started, ready) = std::sync::mpsc::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let producer = {
            let store = store.clone();
            let key = key.clone();
            let loads = loads.clone();
            std::thread::spawn(move || {
                store
                    .preview(&key, || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        started.send(()).unwrap();
                        proceed.recv().unwrap();
                        Ok(Some(image()))
                    })
                    .unwrap()
                    .unwrap()
            })
        };
        ready
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let consumer = {
            let store = Arc::new(store.fork(budget));
            let key = key.clone();
            std::thread::spawn(move || {
                store
                    .preview(&key, || panic!("duplicate preview decoding"))
                    .unwrap()
                    .unwrap()
            })
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let joined = store
                .preview_flights
                .lock()
                .unwrap()
                .iter()
                .any(|(_, flight, _)| flight.strong_count() >= 2);
            if joined {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "second consumer never joined the active producer"
            );
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        let a = producer.join().unwrap();
        let b = consumer.join().unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        let cached = store
            .preview(&key, || panic!("a live lease must remain reusable"))
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&cached, &a));
        store.set_budget(0);
        assert_eq!(store.charges().retained, 0);
        assert_eq!(store.charges().leased_only, a.retained_bytes());
        drop((a, b, cached));
        assert_eq!(store.charges().leased_only, 0);
    }
}

#[test]
fn fork_reuses_completed_preview_without_retaining_it_outside_the_byte_lru() {
    let file = file();
    let key = FileKey::new(file.path());
    let parent = SourceStore::new(1 << 20);
    let preview = parent.preview(&key, || Ok(Some(image()))).unwrap().unwrap();
    let weak = Arc::downgrade(&preview);
    drop(preview);
    let fork = parent.fork(0);
    let reused = fork
        .preview(&key, || panic!("parent already decoded the preview"))
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), &reused));
    parent.close();
    assert_eq!(fork.charges().retained, 0);
    drop(reused);
    assert!(
        weak.upgrade().is_none(),
        "the shared registry must own no preview pixels"
    );
    let mut reloaded = false;
    fork.preview(&key, || {
        reloaded = true;
        Ok(Some(image()))
    })
    .unwrap();
    assert!(reloaded, "unleased, evicted pixels must be loaded afresh");
}

#[test]
fn decoded_previews_obey_byte_lru_and_survive_cache_closure_only_as_leases() {
    let bytes = image().retained_bytes();
    let store = SourceStore::new(bytes * 2);
    let files = [file(), file(), file()];
    let keys = files
        .iter()
        .map(|file| FileKey::new(file.path()))
        .collect::<Vec<_>>();
    let first = store
        .preview(&keys[0], || Ok(Some(image())))
        .unwrap()
        .unwrap();
    let second = store
        .preview(&keys[1], || Ok(Some(image())))
        .unwrap()
        .unwrap();
    let old = Arc::downgrade(&second);
    drop(second);
    store
        .preview(&keys[0], || panic!("cached first preview"))
        .unwrap();
    let third = store
        .preview(&keys[2], || Ok(Some(image())))
        .unwrap()
        .unwrap();
    assert!(
        old.upgrade().is_none(),
        "oldest unprotected preview must be evicted"
    );
    assert_eq!(store.charges().retained, bytes * 2);
    store.close();
    assert_eq!(store.charges().retained, 0);
    assert_eq!(store.charges().leased_only, bytes * 2);
    assert_eq!(first.rgb.get_pixel(0, 0).0, [80, 100, 120]);
    drop((first, third));
    assert_eq!(store.charges().leased_only, 0);
}

#[test]
fn preview_freshness_rejects_changed_files_before_work_and_after_loading() {
    let file = file();
    let key = FileKey::new(file.path());
    let store = SourceStore::new(1 << 20);
    std::fs::write(file.path(), [1, 2]).unwrap();
    assert!(
        store
            .preview(&key, || panic!("stale requests must not perform IO"))
            .is_err()
    );
    let current = FileKey::new(file.path());
    assert!(
        store
            .preview(&current, || {
                std::fs::write(file.path(), [1, 2, 3]).unwrap();
                Ok(Some(image()))
            })
            .is_err()
    );
    assert_eq!(store.charges(), Charges::default());
}
