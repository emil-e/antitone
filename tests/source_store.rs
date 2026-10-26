#![cfg(all(feature = "raw", feature = "project"))]
use antitone::{
    project::{Development, Source, SourceDevelopment, SourceId},
    raw::RawFile,
    scan::{CameraProxy, DevelopedSource, Scan},
    source_store::{Charges, SourceKey, SourceStore},
};
use std::sync::Arc;

#[path = "support/raw_fixture.rs"]
mod fixture;
#[path = "support/fixture_path.rs"]
mod fixture_path;
const WB: [f64; 4] = [1.0; 4];

struct Input {
    file: crate::fixture::FixturePath,
    record: Source,
    development: Development,
    key: SourceKey,
}

impl Input {
    fn new() -> Self {
        let file = fixture::bayer_dng();
        let raw = RawFile::open(file.path()).unwrap();
        let metadata = raw.metadata;
        assert_eq!(metadata.width, fixture::SIDE);
        let area = metadata.image_area().unwrap();
        let mut record = Source {
            id: SourceId(1),
            path: file.path().into(),
            reference_only: false,
            photo: Default::default(),
            local_adjustments: None,
            repairs: Default::default(),
            crop: None,
            orientation: Default::default(),
            development: SourceDevelopment {
                decode: metadata.development(WB),
                lens_correction: None,
                dng_development: metadata.dng_identity(),
                dimensions: [metadata.width, metadata.height],
                active_area: [area.x, area.y, area.width, area.height],
                camera_model: metadata.camera_model(),
                camera_to_ap1: metadata.camera_to_ap1().unwrap().to_cols_array_2d(),
                input_floor: metadata.input_floor().unwrap(),
            },
        };
        let development = metadata.development(WB);
        record.development.decode = development.clone();
        let key = SourceKey::capture(file.path(), &record.development).unwrap();
        Self {
            file,
            record,
            development,
            key,
        }
    }

    fn source(&self) -> Arc<DevelopedSource> {
        Scan::open(self.file.path(), Some(WB))
            .unwrap()
            .source()
            .clone()
    }
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn allocation_snapshot_is_stable_across_consumers_and_charged_once() {
    let input = Input::new();
    let source = input.source();
    let bytes = source.retained_bytes().unwrap();
    let metadata = source.metadata();
    assert!(
        bytes
            >= u64::from(metadata.width) * u64::from(metadata.height) * 8
                + u64::from(metadata.raw_width) * u64::from(metadata.raw_height) * 2
    );
    let scan = Scan::from_source(source.clone());
    let _ = scan.source().sensor_diagnostics().unwrap();
    assert_eq!(scan.source().retained_bytes(), Some(bytes));
    let store = SourceStore::new(bytes);
    let accepted = store
        .publish_source(input.key.clone(), source.clone())
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&accepted, &source));
    store
        .publish_source(input.key.clone(), scan.source().clone())
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: bytes,
            leased_only: 0
        }
    );
    store.set_budget(bytes - 1);
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: bytes
        }
    );
    assert!(Arc::ptr_eq(&store.get(&input.key).unwrap(), &source));
    store.set_budget(0);
    assert_eq!(source.retained_bytes(), Some(bytes));
    assert!(store.get(&input.key).is_some());
}

#[test]
fn native_and_proxy_ownership_compete_in_one_oldest_unprotected_byte_lru() {
    let input = Input::new();
    let scan = Scan::open(input.file.path(), Some(WB)).unwrap();
    let source = scan.source().clone();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let n = source.retained_bytes().unwrap();
    let p = proxy.retained_bytes();
    let store = SourceStore::new(n + p - 1);
    store
        .publish_source(input.key.clone(), source.clone())
        .unwrap()
        .unwrap();
    store
        .publish_proxy(input.key.clone(), proxy.clone())
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: p,
            leased_only: n
        }
    );
    assert!(Arc::ptr_eq(&store.get(&input.key).unwrap(), &source));
    assert_eq!(
        store.charges(),
        Charges {
            retained: n,
            leased_only: p
        }
    );
    assert!(Arc::ptr_eq(&store.get_proxy(&input.key).unwrap(), &proxy));
    assert_eq!(
        store.charges(),
        Charges {
            retained: p,
            leased_only: n
        }
    );
    store.set_budget(0);
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: n + p
        }
    );
    assert!(store.get(&input.key).is_some());
    assert!(store.get_proxy(&input.key).is_some());
    assert_eq!(store.charges().retained, 0);
    store.close();
    assert!(store.get_proxy(&input.key).is_none());
    assert!(store.get(&input.key).is_none());
    drop(proxy);
    drop(source);
    drop(scan);
    assert_eq!(store.charges(), Charges::default());
}

#[test]
fn zero_limits_preserve_protected_proxies_but_never_exempt_full_native_ownership() {
    let input = Input::new();
    let scan = Scan::open(input.file.path(), Some(WB)).unwrap();
    let source = scan.source().clone();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let n = source.retained_bytes().unwrap();
    let p = proxy.retained_bytes();
    let store = SourceStore::new(p);
    store
        .publish_proxy(input.key.clone(), proxy.clone())
        .unwrap()
        .unwrap();
    // The oversized native by-product does not displace the useful proxy.
    store
        .publish_source(input.key.clone(), source.clone())
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: p,
            leased_only: n
        }
    );
    store.protect_proxies(vec![input.key.clone()]);
    store.set_budget(0);
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: n
        }
    );
    assert_eq!(store.protected_bytes(), p);
    drop(proxy);
    assert!(store.get_proxy(&input.key).is_some());
    assert!(store.get(&input.key).is_some());
    assert_eq!(store.charges().retained, 0);
    store.protect_proxies(Vec::new());
    assert_eq!(store.protected_bytes(), 0);
    assert!(store.get_proxy(&input.key).is_none());
}

#[test]
fn stale_keys_and_candidate_close_drop_proxy_ownership_not_active_leases() {
    let input = Input::new();
    let scan = Scan::open(input.file.path(), Some(WB)).unwrap();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let store = SourceStore::new(proxy.retained_bytes());
    store.protect_proxies(vec![input.key.clone()]);
    store
        .publish_proxy(input.key.clone(), proxy.clone())
        .unwrap()
        .unwrap();
    std::fs::write(input.file.path(), b"changed disposable source").unwrap();
    assert!(store.get_proxy(&input.key).is_none());
    assert_eq!(store.protected_bytes(), 0);
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: proxy.retained_bytes()
        }
    );
    assert!(
        store
            .publish_proxy(input.key.clone(), proxy.clone())
            .unwrap()
            .is_none()
    );
    let candidate = SourceStore::new(proxy.retained_bytes());
    candidate.close();
    assert!(candidate.get_proxy(&input.key).is_none());
    drop(proxy);
    assert_eq!(store.charges(), Charges::default());
}

#[test]
fn concurrent_mixed_publications_never_exceed_one_optional_limit() {
    use std::sync::Barrier;
    let input = Input::new();
    let scan = Scan::open(input.file.path(), Some(WB)).unwrap();
    let source = scan.source().clone();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let budget = source.retained_bytes().unwrap() + proxy.retained_bytes() - 1;
    let store = Arc::new(SourceStore::new(budget));
    let barrier = Arc::new(Barrier::new(5));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let store = store.clone();
            let source = source.clone();
            let proxy = proxy.clone();
            let key = input.key.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..16 {
                    store
                        .publish_source(key.clone(), source.clone())
                        .unwrap()
                        .unwrap();
                    store
                        .publish_proxy(key.clone(), proxy.clone())
                        .unwrap()
                        .unwrap();
                    assert!(store.charges().retained <= budget);
                }
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(store.charges().retained <= budget);
    store.close();
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: source.retained_bytes().unwrap() + proxy.retained_bytes()
        }
    );
    assert!(
        store
            .publish_proxy(input.key.clone(), proxy)
            .unwrap()
            .is_none()
    );
}

#[test]
fn proxy_basis_rejection_does_not_replace_a_working_entry() {
    let input = Input::new();
    let scan = Scan::open(input.file.path(), Some(WB)).unwrap();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    let store = SourceStore::new(proxy.retained_bytes());
    store
        .publish_proxy(input.key.clone(), proxy.clone())
        .unwrap()
        .unwrap();
    let mut expected = input.record.development.clone();
    expected.camera_to_ap1[0][0] += 0.01;
    let wrong = SourceKey::capture(input.file.path(), &expected).unwrap();
    assert!(store.publish_proxy(wrong, proxy.clone()).is_err());
    assert!(Arc::ptr_eq(&store.get_proxy(&input.key).unwrap(), &proxy));
}

#[test]
fn exact_keys_exclude_grade_identity_and_fail_closed_without_stats() {
    let mut input = Input::new();
    let before = input.key.clone();
    input.record.id = SourceId(99);
    input.record.reference_only = true;
    input.record.photo.exposure = 2.0;
    assert_eq!(
        SourceKey::capture(input.file.path(), &input.record.development).unwrap(),
        before
    );
    let directory = tempfile::tempdir().unwrap();
    assert!(
        SourceKey::capture(
            &directory.path().join("missing.dng"),
            &input.record.development,
        )
        .is_none()
    );
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias.dng");
        std::os::unix::fs::symlink(input.file.path(), &alias).unwrap();
        assert_eq!(
            SourceKey::capture(&alias, &input.record.development).unwrap(),
            before
        );
    }
}

#[test]
fn mismatched_native_basis_geometry_wb_and_runtime_never_replace_working_entries() {
    let input = Input::new();
    let source = input.source();
    let store = SourceStore::new(10);
    store
        .publish(input.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    let mut wrong_sources = Vec::new();
    let mut source_key = input.record.development.clone();
    source_key.dimensions[0] += 1;
    wrong_sources.push(source_key);
    let mut source_key = input.record.development.clone();
    source_key.active_area[0] += 1;
    wrong_sources.push(source_key);
    let mut source_key = input.record.development.clone();
    source_key.camera_model.push_str(" wrong");
    wrong_sources.push(source_key);
    let mut source_key = input.record.development.clone();
    source_key.camera_to_ap1[0][0] += 0.01;
    wrong_sources.push(source_key);
    let mut source_key = input.record.development.clone();
    source_key.input_floor *= 2.0;
    wrong_sources.push(source_key);
    for expected in wrong_sources {
        let key = SourceKey::capture(input.file.path(), &expected).unwrap();
        assert!(store.publish(key, source.clone(), 10).is_err());
    }
    for field in 0..5 {
        let mut development = input.development.clone();
        match field {
            0 => development.fixed_wb[0] = 2.0,
            1 => development.backend_version.push_str(" wrong"),
            2 => development.contract.push_str(" wrong"),
            3 => development.inversion_space.push_str(" wrong"),
            _ => development.working_space.push_str(" wrong"),
        }
        let key = SourceKey::capture(
            input.file.path(),
            &SourceDevelopment {
                decode: development,
                ..input.record.development.clone()
            },
        )
        .unwrap();
        assert!(store.publish(key, source.clone(), 10).is_err());
    }
    assert_eq!(
        store.charges(),
        Charges {
            retained: 10,
            leased_only: 0
        }
    );
    assert!(Arc::ptr_eq(&store.get(&input.key).unwrap(), &source));
}

#[test]
fn byte_lru_evicts_ownership_not_active_leases_and_zero_limits_keep_live_reuse() {
    let a = Input::new();
    let b = Input::new();
    let c = Input::new();
    let first = a.source();
    let second = b.source();
    let third = c.source();
    let store = SourceStore::new(20);
    store
        .publish(a.key.clone(), first.clone(), 10)
        .unwrap()
        .unwrap();
    store
        .publish(b.key.clone(), second.clone(), 10)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&store.get(&a.key).unwrap(), &first));
    store
        .publish(c.key.clone(), third.clone(), 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: 20,
            leased_only: 10
        }
    );
    let dead = Arc::downgrade(&second);
    drop(second);
    assert!(
        dead.upgrade().is_none(),
        "B was oldest and should have lost optional ownership"
    );
    assert!(store.get(&b.key).is_none());
    assert!(store.get(&a.key).is_some());
    store.set_budget(0);
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: 20
        }
    );
    let leased = store.get(&a.key).unwrap();
    assert!(Arc::ptr_eq(&leased, &first));
    drop(leased);
    drop(first);
    drop(third);
    assert_eq!(store.charges(), Charges::default());
    assert!(store.get(&a.key).is_none());
}

#[test]
fn oversized_weak_only_publication_does_not_evict_and_can_be_readmitted_later() {
    let a = Input::new();
    let b = Input::new();
    let first = a.source();
    let second = b.source();
    let store = SourceStore::new(10);
    store
        .publish(a.key.clone(), first.clone(), 10)
        .unwrap()
        .unwrap();
    store
        .publish(b.key.clone(), second.clone(), 20)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: 10,
            leased_only: 20
        }
    );
    drop(first);
    assert!(store.get(&a.key).is_some());
    store.set_budget(30);
    store
        .publish(b.key.clone(), second.clone(), 20)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: 30,
            leased_only: 0
        }
    );
    drop(second);
    assert!(store.get(&b.key).is_some());
}

#[test]
fn demanded_weak_hits_are_readmitted_only_when_the_explicit_limit_fits() {
    let input = Input::new();
    let source = input.source();
    let store = SourceStore::new(0);
    store
        .publish(input.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: 10
        }
    );
    store.set_budget(9);
    assert!(store.get(&input.key).is_some());
    assert_eq!(store.charges().retained, 0);
    store.set_budget(10);
    assert!(store.get(&input.key).is_some());
    assert_eq!(
        store.charges(),
        Charges {
            retained: 10,
            leased_only: 0
        }
    );
    drop(source);
    assert!(store.get(&input.key).is_some());
}

#[test]
fn concurrent_duplicate_publications_and_closed_store_lookup_preserve_one_charge() {
    use std::sync::Barrier;
    let input = Input::new();
    let source = input.source();
    let store = Arc::new(SourceStore::new(10));
    let barrier = Arc::new(Barrier::new(5));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let store = store.clone();
            let source = source.clone();
            let key = input.key.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..16 {
                    let accepted = store
                        .publish(key.clone(), source.clone(), 10)
                        .unwrap()
                        .unwrap();
                    assert!(Arc::ptr_eq(&accepted, &source));
                    assert!(Arc::ptr_eq(&store.get(&key).unwrap(), &source));
                }
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        store.charges(),
        Charges {
            retained: 10,
            leased_only: 0
        }
    );
    store.close();
    assert!(store.get(&input.key).is_none());
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: 10
        }
    );
    drop(source);
    assert_eq!(store.charges(), Charges::default());
}

#[test]
fn duplicate_completions_share_one_charge_and_reject_alias_or_reweighting() {
    let input = Input::new();
    let other = Input::new();
    let first = input.source();
    let duplicate = input.source();
    let store = SourceStore::new(20);
    store
        .publish(input.key.clone(), first.clone(), 10)
        .unwrap()
        .unwrap();
    let old = Arc::downgrade(&duplicate);
    let retained = store
        .publish(input.key.clone(), duplicate, 11)
        .unwrap()
        .unwrap();
    assert!(old.upgrade().is_none());
    assert!(Arc::ptr_eq(&retained, &first));
    assert_eq!(
        store.charges(),
        Charges {
            retained: 10,
            leased_only: 0
        }
    );
    assert!(store.publish(other.key.clone(), first.clone(), 10).is_err());
    assert!(store.publish(input.key.clone(), first.clone(), 11).is_err());
    assert!(store.publish(input.key.clone(), first.clone(), 0).is_err());
    assert_eq!(
        store.charges(),
        Charges {
            retained: 10,
            leased_only: 0
        }
    );
}

#[test]
fn changed_files_reject_prework_publication_and_hits_without_losing_leased_accounting() {
    let input = Input::new();
    let source = input.source();
    let store = SourceStore::new(10);
    store
        .publish(input.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    input.file.append(b"changed");
    assert!(store.get(&input.key).is_none());
    assert!(
        store
            .publish(input.key.clone(), source.clone(), 10)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: 10
        }
    );
    drop(source);
    assert_eq!(store.charges(), Charges::default());
}

#[cfg(unix)]
#[test]
fn restored_mtime_and_symlink_retargeting_do_not_make_old_keys_current() {
    use std::io::Write;
    let input = Input::new();
    let source = input.source();
    let store = SourceStore::new(10);
    store
        .publish(input.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(input.file.path())
        .unwrap();
    let modified = file.metadata().unwrap().modified().unwrap();
    file.write_all(b"MM").unwrap();
    file.set_modified(modified).unwrap();
    assert!(store.get(&input.key).is_none());

    let a = Input::new();
    let b = Input::new();
    let directory = tempfile::tempdir().unwrap();
    let alias = directory.path().join("alias.dng");
    std::os::unix::fs::symlink(a.file.path(), &alias).unwrap();
    let captured = SourceKey::capture(&alias, &a.record.development).unwrap();
    let source = a.source();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(b.file.path(), &alias).unwrap();
    assert!(
        store
            .publish(captured.clone(), source.clone(), 10)
            .unwrap()
            .is_none()
    );
    assert!(store.get(&captured).is_none());
    store
        .publish(a.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&store.get(&a.key).unwrap(), &source));
}

#[test]
fn candidate_close_rejects_late_jobs_without_touching_accepted_store_or_leases() {
    let input = Input::new();
    let source = input.source();
    let accepted = SourceStore::new(10);
    let candidate = SourceStore::new(10);
    accepted
        .publish(input.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    candidate
        .publish(input.key.clone(), source.clone(), 10)
        .unwrap()
        .unwrap();
    candidate.close();
    assert!(candidate.get(&input.key).is_none());
    assert!(
        candidate
            .publish(input.key.clone(), source.clone(), 10)
            .unwrap()
            .is_none()
    );
    assert!(Arc::ptr_eq(&accepted.get(&input.key).unwrap(), &source));
    accepted.evict(&input.key);
    assert_eq!(
        accepted.charges(),
        Charges {
            retained: 0,
            leased_only: 10
        }
    );
    assert!(accepted.get(&input.key).is_some());
    assert!(source.camera_image().unwrap().pixel_samples(8, 8).is_ok());
}

#[test]
fn aggregate_overflow_is_transactional_including_evicted_but_leased_storage() {
    let a = Input::new();
    let b = Input::new();
    let first = a.source();
    let second = b.source();
    let store = SourceStore::new(u64::MAX);
    store
        .publish(a.key.clone(), first.clone(), u64::MAX)
        .unwrap()
        .unwrap();
    store.set_budget(0);
    assert!(store.publish(b.key.clone(), second.clone(), 1).is_err());
    assert_eq!(
        store.charges(),
        Charges {
            retained: 0,
            leased_only: u64::MAX
        }
    );
    drop(first);
    assert_eq!(store.charges(), Charges::default());
    store.publish(b.key.clone(), second, 1).unwrap().unwrap();
    assert_eq!(store.charges(), Charges::default());
}
