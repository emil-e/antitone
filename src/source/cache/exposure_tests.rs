use super::*;
use crate::{
    exposure_reference::{ReferenceExposure, StartingExposure},
    processing::InputSettings,
    project::SourceDevelopment,
    raw::{RawFile, fixture},
    scan::{CameraProxy, Scan},
};
use std::sync::OnceLock;

fn matched(ev: f64) -> ReferenceExposure {
    ReferenceExposure::Matched(StartingExposure {
        ev,
        samples: 1024,
        spread: 0.01,
    })
}

#[test]
fn independent_exposure_values_share_the_same_cached_native_and_proxy_pixels() {
    let file = fixture::bayer_dng();
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let original = scan.source().clone();
    let pixels = Arc::new(CameraPixels::from_scan(&scan).unwrap());
    let development =
        SourceDevelopment::from_metadata(&original.metadata(), original.fixed_wb()).unwrap();
    let key = SourceKey::capture(file.path(), &development).unwrap();
    let cache = Arc::new(SourceCache::new(64 << 20));
    cache.publish_source(key.clone(), original.clone()).unwrap();
    cache.publish_proxy(key.clone(), pixels.clone()).unwrap();
    let first_value = matched(1.);
    let second_value = matched(2.);
    let first = crate::photo_preparation::native(
        None,
        &OnceLock::from(first_value.clone()),
        InputSettings::Digital,
        &original,
    )
    .unwrap();
    let second = crate::photo_preparation::native(
        None,
        &OnceLock::from(second_value.clone()),
        InputSettings::Digital,
        &original,
    )
    .unwrap();
    let first_proxy = CameraProxy::new(pixels.clone(), Some(first_value));
    let second_proxy = CameraProxy::new(pixels.clone(), Some(second_value));
    assert!(Arc::ptr_eq(&first.source, &second.source));
    assert!(Arc::ptr_eq(&first_proxy.data, &second_proxy.data));
    assert!(Arc::ptr_eq(&cache.get(&key).unwrap(), &original));
    assert!(Arc::ptr_eq(&cache.get_proxy(&key).unwrap(), &pixels));
    for (prepared, proxy, gain) in [(&first, &first_proxy, 2.), (&second, &second_proxy, 4.)] {
        assert_eq!(
            prepared
                .scan()
                .positive_matrix(&InputSettings::Digital)
                .unwrap(),
            scan.camera_to_ap1 * gain
        );
        assert_eq!(
            proxy.positive_matrix(&InputSettings::Digital).unwrap(),
            scan.camera_to_ap1 * gain
        );
        assert!(
            prepared.proxy.is_none(),
            "a recorded value needs no new sampling"
        );
    }
    assert_eq!(
        first
            .scan()
            .positive_matrix(&InputSettings::Digital)
            .unwrap(),
        scan.camera_to_ap1 * 2.,
        "a later interpretation cannot alter an earlier consumer"
    );
}

#[test]
fn recorded_exposure_outlives_its_cache_without_retaining_optional_pixels() {
    let file = fixture::bayer_dng();
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let exposures = crate::exposure_metadata::Store::default();
    let id = crate::project::SourceId(1);
    let reference = exposures.memo(id, file.path());
    assert!(Arc::ptr_eq(&reference, &exposures.memo(id, file.path())));
    reference.set(matched(1.75)).unwrap();
    let owner = Arc::new(SourceCache::new(0));
    let weak = Arc::downgrade(&owner);
    drop(owner);
    assert!(weak.upgrade().is_none());

    let prepared =
        crate::photo_preparation::native(None, &reference, InputSettings::Digital, scan.source())
            .unwrap();
    assert_eq!(
        prepared
            .scan()
            .positive_matrix(&InputSettings::Digital)
            .unwrap(),
        scan.camera_to_ap1 * 1.75_f64.exp2()
    );
    assert!(prepared.proxy.is_none());
}
