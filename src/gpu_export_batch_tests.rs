use super::*;
use crate::gpu_export::reader::{JobReader, TileReader};
use std::{cell::Cell, rc::Rc};
fn pixels(origins: &[Origin]) -> Vec<Pixel> {
    origins
        .iter()
        .flat_map(|o| [[o[0] as f64, -1., 4., 7.]; 2])
        .collect()
}

#[test]
fn pairs_only_planned_misses_keep_cache_order_stop_checks_and_one_pending_tile() {
    let checks = Rc::new(Cell::new(0));
    let counter = checks.clone();
    let mut reader = JobReader::new(
        PairLoader::new(|o: &[Origin]| Ok(pixels(o)), 2, true),
        move || {
            counter.set(counter.get() + 1);
            Ok(())
        },
        true,
    );
    assert_eq!(reader.read([0, 0]).unwrap().as_ref(), pixels(&[[0, 0]]));
    let origins = [[0, 0], [1, 0], [2, 0], [3, 0]];
    reader.prepare(origins.into_iter()).unwrap();
    for origin in origins {
        assert_eq!(reader.read(origin).unwrap().as_ref(), pixels(&[origin]));
    }
    reader.finish().unwrap();
    assert_eq!(checks.get(), 10);
    assert_eq!(reader.requests, 5);
    let loader = reader.into_loader();
    assert_eq!(loader.reads, 4);
    assert_eq!(loader.dispatches, 3);
    assert!(loader.pending.is_none());
    assert!(loader.misses.is_empty());
}

#[test]
fn serial_mode_never_looks_ahead_and_incomplete_or_misordered_batches_reject() {
    let mut serial = PairLoader::new(
        |o: &[Origin]| {
            assert_eq!(o.len(), 1);
            Ok(pixels(o))
        },
        2,
        false,
    );
    serial.prepare([[0, 0], [1, 0]].into_iter()).unwrap();
    serial.load([0, 0]).unwrap();
    serial.finish().unwrap();
    assert_eq!(serial.reads, 1);
    assert_eq!(serial.dispatches, 1);
    let mut pair = PairLoader::new(|o: &[Origin]| Ok(pixels(o)), 2, true);
    pair.prepare([[0, 0], [1, 0]].into_iter()).unwrap();
    pair.load([0, 0]).unwrap();
    assert!(pair.pending.is_some());
    assert!(pair.finish().is_err());
    assert!(pair.prepare(std::iter::empty()).is_err());
    assert!(pair.load([2, 0]).is_err());
    let mut pair = PairLoader::new(|o: &[Origin]| Ok(pixels(o)), 2, true);
    pair.prepare([[0, 0], [1, 0]].into_iter()).unwrap();
    assert!(pair.load([1, 0]).is_err());
    assert_eq!(pair.dispatches, 0);
}

#[test]
fn failed_or_malformed_gpu_batches_publish_no_pending_tile() {
    for malformed in [false, true] {
        let mut pair = PairLoader::new(
            move |_: &[Origin]| {
                if malformed {
                    Ok(vec![[0.; 4]; 3])
                } else {
                    Err(Error::Gpu("batch failed".into()))
                }
            },
            2,
            true,
        );
        pair.prepare([[0, 0], [1, 0]].into_iter()).unwrap();
        assert!(pair.load([0, 0]).is_err());
        assert!(pair.pending.is_none());
    }
}

#[test]
fn cancellation_before_or_after_batch_and_panics_release_job_ownership() {
    for stop in [1, 2, 3] {
        let owner = Rc::new(());
        let weak = Rc::downgrade(&owner);
        let checks = Cell::new(0);
        let calls = Rc::new(Cell::new(0));
        let work = calls.clone();
        {
            let load = PairLoader::new(
                move |o: &[Origin]| {
                    let _ = &owner;
                    work.set(work.get() + 1);
                    Ok(pixels(o))
                },
                2,
                true,
            );
            let mut reader = JobReader::new(
                load,
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == stop {
                        Err(Error::Export("cancelled".into()))
                    } else {
                        Ok(())
                    }
                },
                true,
            );
            reader.prepare([[0, 0], [1, 0]].into_iter()).unwrap();
            let first = reader.read([0, 0]);
            if first.is_ok() {
                assert!(reader.read([1, 0]).is_err());
            } else {
                assert!(first.is_err());
            }
        }
        assert!(weak.upgrade().is_none());
        assert_eq!(calls.get(), u32::from(stop != 1));
    }
    let owner = Rc::new(());
    let weak = Rc::downgrade(&owner);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut pair = PairLoader::new(
                move |_: &[Origin]| -> Result<Vec<Pixel>, Error> {
                    let _ = &owner;
                    panic!("GPU callback panic")
                },
                2,
                true,
            );
            pair.prepare([[0, 0], [1, 0]].into_iter()).unwrap();
            let _ = pair.load([0, 0]);
        }))
        .is_err()
    );
    assert!(weak.upgrade().is_none());
}
