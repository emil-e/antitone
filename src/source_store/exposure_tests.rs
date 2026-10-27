use super::*;
use crate::{
    exposure_reference::{ReferenceExposure, StartingExposure, UnavailableReference},
    processing::InputSettings,
    raw::{RawFile, fixture},
    scan::Scan,
};

fn matched(ev: f64) -> ReferenceExposure {
    ReferenceExposure::Matched(StartingExposure {
        ev,
        samples: 1024,
        spread: 0.01,
    })
}

#[test]
fn canonical_reference_rebinds_cached_pixels_without_mutating_existing_leases() {
    let file = fixture::bayer_dng();
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let original = scan.source().clone();
    let stale = ReferenceExposure::Unavailable(UnavailableReference::ReadFailed);
    original.reference_exposure.set(stale.clone()).unwrap();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let development =
        SourceDevelopment::from_metadata(&original.metadata(), original.fixed_wb()).unwrap();
    let key = SourceKey::capture(file.path(), &development).unwrap();
    let store = SourceStore::new(64 << 20);
    store.publish_source(key.clone(), original.clone()).unwrap();
    store.publish_proxy(key.clone(), proxy.clone()).unwrap();
    let canonical = matched(2.25);
    store.exposure_memo(&key).set(canonical.clone()).unwrap();

    let prepared = store
        .prepare_source(InputSettings::Digital, &original)
        .unwrap();
    let resolved_proxy = store
        .prepare_proxy(&key, InputSettings::Digital, &proxy)
        .unwrap();
    assert_eq!(prepared.source.reference_exposure.get(), Some(&canonical));
    assert_eq!(resolved_proxy.reference_exposure.get(), Some(&canonical));
    assert_eq!(original.reference_exposure.get(), Some(&stale));
    assert_eq!(proxy.reference_exposure.get(), Some(&stale));
    assert!(Arc::ptr_eq(&proxy.pixels, &resolved_proxy.pixels));
    assert!(Arc::ptr_eq(&store.get(&key).unwrap(), &prepared.source));
    assert!(Arc::ptr_eq(
        &store.get_proxy(&key).unwrap(),
        &resolved_proxy
    ));
    assert!(
        prepared.proxy.is_none(),
        "a restored result needs no reference extraction or sampling"
    );
    assert!(store.preview_flights.lock().unwrap().is_empty());
    assert_eq!(
        prepared
            .source
            .positive_matrix(&InputSettings::Digital)
            .unwrap(),
        scan.camera_to_ap1 * 2.25_f64.exp2()
    );
    assert_eq!(
        resolved_proxy
            .positive_matrix(&InputSettings::Digital)
            .unwrap(),
        scan.camera_to_ap1 * 2.25_f64.exp2()
    );
}

#[test]
fn cache_fork_shares_unresolved_metering_cells_and_preserves_results_without_pixels() {
    let file = fixture::bayer_dng();
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let development =
        SourceDevelopment::from_metadata(&scan.source().metadata(), scan.source().fixed_wb())
            .unwrap();
    let key = SourceKey::capture(file.path(), &development).unwrap();
    let store = SourceStore::new(0);
    let memo = store.exposure_memo(&key);
    let fork = store.fork(0);
    assert!(
        Arc::ptr_eq(&memo, &fork.exposure_memo(&key)),
        "unresolved work must share its synchronization cell"
    );
    memo.set(matched(1.75)).unwrap();
    store.close();
    let prepared = fork
        .prepare_source(InputSettings::Digital, scan.source())
        .unwrap();
    assert_eq!(
        prepared
            .source
            .positive_matrix(&InputSettings::Digital)
            .unwrap(),
        scan.camera_to_ap1 * 1.75_f64.exp2()
    );
    assert!(prepared.proxy.is_none());
    assert_eq!(fork.charges().retained, 0);
    assert!(fork.preview_flights.lock().unwrap().is_empty());
}

#[test]
fn cpu_only_publication_applies_known_defaults_before_pending_preview_consumption() {
    let file = fixture::bayer_dng();
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let source = scan.source().clone();
    let stale = ReferenceExposure::Unavailable(UnavailableReference::ReadFailed);
    source.reference_exposure.set(stale.clone()).unwrap();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let development =
        SourceDevelopment::from_metadata(&source.metadata(), source.fixed_wb()).unwrap();
    let key = SourceKey::capture(file.path(), &development).unwrap();
    let store = SourceStore::new(64 << 20);
    store.publish_source(key.clone(), source.clone()).unwrap();
    store.publish_proxy(key.clone(), proxy.clone()).unwrap();
    let canonical = matched(2.25);
    store.exposure_memo(&key).set(canonical.clone()).unwrap();

    // Neither publication invokes preparation: this is the CPU-only warming boundary.
    let published_source = store
        .publish_source(key.clone(), source.clone())
        .unwrap()
        .unwrap();
    let published_proxy = store
        .publish_proxy(key.clone(), proxy.clone())
        .unwrap()
        .unwrap();
    assert_eq!(published_source.reference_exposure.get(), Some(&canonical));
    assert_eq!(published_proxy.reference_exposure.get(), Some(&canonical));
    assert_eq!(source.reference_exposure.get(), Some(&stale));
    assert_eq!(proxy.reference_exposure.get(), Some(&stale));
    assert!(Arc::ptr_eq(&proxy.pixels, &published_proxy.pixels));
    assert!(store.preview_flights.lock().unwrap().is_empty());
    // Repeated normal publication must preserve canonical lease identity and accounting.
    let again = store
        .publish_source(key.clone(), published_source.clone())
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&again, &published_source));
    assert!(Arc::ptr_eq(
        &store
            .publish_proxy(key, published_proxy.clone())
            .unwrap()
            .unwrap(),
        &published_proxy
    ));
}
