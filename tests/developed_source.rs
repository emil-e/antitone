#![cfg(all(feature = "raw", feature = "project"))]

use antitone::{
    flat_field::FlatField,
    raw::{RawFile, Rect},
    scan::{DevelopedSource, Scan},
};
use glam::{DMat3, DVec3};
use std::sync::{Arc, Barrier};

#[path = "support/raw_fixture.rs"]
mod fixture;

const WB: [f64; 4] = [1.0; 4];

fn region() -> Rect {
    Rect {
        x: 8,
        y: 8,
        width: 16,
        height: 16,
    }
}

fn codes(source: &DevelopedSource) -> Vec<[u16; 4]> {
    let image = source.camera_image().unwrap();
    (0..image.height)
        .flat_map(|y| (0..image.width).map(move |x| (x, y)))
        .map(|(x, y)| image.pixel_codes(x, y).unwrap())
        .collect()
}

#[test]
fn frozen_native_codes_metadata_and_capture_match_unique_development() {
    let file = fixture::bayer_dng();
    let mut raw = RawFile::open(file.path()).unwrap();
    assert_eq!(
        (raw.metadata.width, raw.metadata.height),
        (fixture::SIDE, fixture::SIDE)
    );
    let metadata = raw.metadata;
    let capture = raw.region_sensor_diagnostics(region()).unwrap();
    let whole = raw.unpack().unwrap();
    let image = raw.develop_camera(WB).unwrap();
    let expected: Vec<_> = (0..image.height)
        .flat_map(|y| (0..image.width).map(move |x| (x, y)))
        .map(|(x, y)| image.pixel_codes(x, y).unwrap())
        .collect();
    let view = (
        image.width,
        image.height,
        image.maximum,
        image.data_maximum,
        image.warnings,
        image.pre_mul,
    );
    drop(raw);

    let scan = Scan::open(file.path(), Some(WB)).unwrap();
    let source = scan.source();
    assert_eq!(format!("{:?}", source.metadata()), format!("{metadata:?}"));
    assert_eq!(source.fixed_wb(), WB);
    assert_eq!(source.camera_model(), scan.camera_model);
    assert_eq!(source.camera_to_ap1(), scan.camera_to_ap1);
    assert_eq!(source.area(), scan.area);
    assert_eq!(source.input_floor(), scan.input_floor);
    assert_eq!(source.region_sensor_diagnostics(region()).unwrap(), capture);
    assert_eq!(source.sensor_diagnostics().unwrap(), whole);
    assert_eq!(codes(source), expected);
    let image = source.camera_image().unwrap();
    assert_eq!(
        (
            image.width,
            image.height,
            image.maximum,
            image.data_maximum,
            image.warnings,
            image.pre_mul
        ),
        view
    );
    assert!(
        source
            .region_sensor_diagnostics(Rect {
                width: 0,
                ..region()
            })
            .is_err()
    );
    assert_eq!(source.region_sensor_diagnostics(region()).unwrap(), capture);
    assert_eq!(codes(source), expected);
}

#[test]
fn leases_keep_native_storage_alive_with_independent_correction_contexts() {
    let file = fixture::bayer_dng();
    let first = Scan::open(file.path(), Some(WB)).unwrap();
    let source = first.source().clone();
    let weak = Arc::downgrade(&source);
    let original = first.pixel(16, 16).unwrap();
    let mut corrected = Scan::from_source(source.clone());
    let mut plain = Scan::from_source(source.clone());
    assert!(Arc::ptr_eq(first.source(), corrected.source()));
    let area = source.area();
    let field = Arc::new(
        FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![DVec3::splat(2.0); 4],
        )
        .unwrap(),
    );
    corrected.set_flat_field(Some(field)).unwrap();
    assert_eq!(corrected.pixel(16, 16).unwrap(), original * 2.0);
    assert_eq!(plain.pixel(16, 16).unwrap(), original);
    assert_eq!(corrected.effective_floor(), 2.0 * source.input_floor());
    assert_eq!(plain.effective_floor(), source.input_floor());
    let mean = plain.reference(region()).unwrap();
    assert_eq!(corrected.reference(region()).unwrap(), mean * 2.0);
    let fresh = Scan::from_source(corrected.source().clone());
    assert!(fresh.flat_field().is_none());
    assert_eq!(fresh.pixel(16, 16).unwrap(), original);
    let (_, _, preview) = plain.preview_source(8).unwrap();
    assert_eq!(fresh.preview_source(8).unwrap().2, preview);
    let invalid = Arc::new(
        FlatField::from_nodes([area.width, area.height, 1, 1], [2, 2], vec![DVec3::ONE; 4])
            .unwrap(),
    );
    assert!(corrected.set_flat_field(Some(invalid)).is_err());
    assert_eq!(corrected.pixel(16, 16).unwrap(), original * 2.0);
    drop(first);
    drop(source);
    drop(plain);
    drop(fresh);
    assert!(weak.upgrade().is_some());
    assert_eq!(corrected.pixel(16, 16).unwrap(), original * 2.0);
    drop(corrected);
    assert!(weak.upgrade().is_none());
}

#[test]
fn synchronized_handoff_concurrent_readers_and_receiver_final_release() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<DevelopedSource>();
    send_sync::<Scan>();
    let file = fixture::bayer_dng();
    let producer = Scan::open(file.path(), Some(WB)).unwrap();
    let source = producer.source().clone();
    let expected = codes(&source);
    let capture = source.region_sensor_diagnostics(region()).unwrap();
    let weak = Arc::downgrade(&source);
    let barrier = Arc::new(Barrier::new(3));
    let (send, receive) = std::sync::mpsc::channel::<Arc<DevelopedSource>>();
    let (finished, wait) = std::sync::mpsc::channel();
    let reader = source.clone();
    let reader_barrier = barrier.clone();
    let a = std::thread::spawn(move || {
        reader_barrier.wait();
        for _ in 0..8 {
            assert_eq!(reader.region_sensor_diagnostics(region()).unwrap(), capture);
            assert!(
                reader
                    .camera_image()
                    .unwrap()
                    .sample_mean(region(), DMat3::IDENTITY)
                    .is_ok()
            );
        }
        let actual = codes(&reader);
        drop(reader);
        finished.send(()).unwrap();
        actual
    });
    let final_barrier = barrier.clone();
    let b = std::thread::spawn(move || {
        let source = receive.recv().unwrap();
        final_barrier.wait();
        let mut consumer = Scan::from_source(source.clone());
        for _ in 0..8 {
            assert_eq!(source.region_sensor_diagnostics(region()).unwrap(), capture);
            assert!(consumer.reference(region()).is_ok());
        }
        let actual = codes(&source);
        wait.recv().unwrap();
        drop(consumer);
        drop(source);
        actual
    });
    drop(producer);
    send.send(source).unwrap();
    drop(send);
    barrier.wait();
    assert_eq!(a.join().unwrap(), expected);
    assert_eq!(b.join().unwrap(), expected);
    assert!(weak.upgrade().is_none());
}
