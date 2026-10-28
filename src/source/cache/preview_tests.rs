use super::*;

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
fn decoded_previews_obey_byte_lru_and_survive_their_cache_as_leases() {
    let bytes = image().retained_bytes();
    let store = Arc::new(SourceCache::new(bytes * 2));
    let files = [file(), file(), file()];
    let keys: Vec<_> = files.iter().map(|file| FileKey::new(file.path())).collect();
    let first = image();
    store
        .publish_preview(keys[0].clone(), first.clone())
        .unwrap();
    let second = image();
    let old = Arc::downgrade(&second);
    store.publish_preview(keys[1].clone(), second).unwrap();
    assert!(Arc::ptr_eq(&store.get_preview(&keys[0]).unwrap(), &first));
    let third = image();
    store
        .publish_preview(keys[2].clone(), third.clone())
        .unwrap();
    assert!(
        old.upgrade().is_none(),
        "oldest unprotected preview must be evicted"
    );
    assert_eq!(store.charges().retained, bytes * 2);
    let owner = Arc::downgrade(&store);
    drop(store);
    assert!(
        owner.upgrade().is_none(),
        "optional access must not retain the cache"
    );
    assert_eq!(first.rgb.get_pixel(0, 0).0, [80, 100, 120]);
    assert_eq!(third.rgb.dimensions(), (16, 8));
}
