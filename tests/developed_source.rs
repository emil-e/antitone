#![cfg(all(feature = "raw", feature = "project"))]

use antitone::{
    flat_field::{FlatField, sample_camera_light},
    raw::{RawFile, Rect},
    sampling::camera::reference,
    scan::DevelopedSource,
};
use glam::{DMat3, DVec3};
use std::sync::{Arc, Barrier};

#[path = "support/raw_fixture.rs"]
mod fixture;
#[path = "support/fixture_path.rs"]
mod fixture_path;

const WB: [f64; 4] = [1.0; 4];

fn region() -> Rect {
    Rect {
        x: 8,
        y: 8,
        width: 16,
        height: 16,
    }
}

fn codes(source: &DevelopedSource) -> Vec<[f32; 4]> {
    let image = source.camera_image().unwrap();
    (0..image.height)
        .flat_map(|y| (0..image.width).map(move |x| (x, y)))
        .map(|(x, y)| image.pixel_samples(x, y).unwrap())
        .collect()
}

#[test]
fn frozen_pixels_and_metadata_match_unique_development() {
    let file = fixture::bayer_dng();
    let mut raw = RawFile::open(file.path()).unwrap();
    assert_eq!(
        (raw.metadata.width, raw.metadata.height),
        (fixture::SIDE, fixture::SIDE)
    );
    let metadata = raw.metadata;
    let image = raw.develop_camera(WB).unwrap();
    let expected: Vec<_> = (0..image.height)
        .flat_map(|y| (0..image.width).map(move |x| (x, y)))
        .map(|(x, y)| image.pixel_samples(x, y).unwrap())
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

    let source = DevelopedSource::open(file.path(), Some(WB)).unwrap();
    assert_eq!(format!("{:?}", source.metadata()), format!("{metadata:?}"));
    assert_eq!(source.white_balance(), WB);
    assert_eq!(source.camera_model(), metadata.camera_model());
    assert_eq!(source.camera_to_ap1(), metadata.camera_to_ap1().unwrap());
    assert_eq!(source.area(), metadata.image_area().unwrap());
    assert_eq!(codes(&source), expected);
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
}

#[test]
fn leases_keep_native_storage_alive_with_independent_correction_contexts() {
    let file = fixture::bayer_dng();
    let source = DevelopedSource::open(file.path(), Some(WB)).unwrap();
    let weak = Arc::downgrade(&source);
    let corrected_lease = source.clone();
    let original = source.pixel(16, 16).unwrap();
    let area = source.area();
    let field = FlatField::from_nodes(
        [area.x, area.y, area.width, area.height],
        [2, 2],
        vec![DVec3::splat(2.0); 4],
    )
    .unwrap();
    let point = [16, 16];
    let baseline_proxy = source.preview_source(8).unwrap();
    assert_eq!(
        sample_camera_light(&corrected_lease, Some(&field), point).unwrap(),
        original * 2.0
    );
    assert_eq!(source.pixel(16, 16).unwrap(), original);
    let mean = reference(&source, None, region()).unwrap();
    let corrected_mean = reference(&corrected_lease, Some(&field), region()).unwrap();
    // Plain and corrected references group their ordinary sums differently.
    assert!((corrected_mean - mean * 2.0).abs().max_element() < 1e-12);
    let outside = FlatField::from_nodes(
        [area.width, area.height, 1, 1],
        [2, 2],
        vec![DVec3::splat(3.); 4],
    )
    .unwrap();
    assert_eq!(
        sample_camera_light(&corrected_lease, Some(&outside), point).unwrap(),
        original
    );
    assert_eq!(source.preview_source(8).unwrap(), baseline_proxy);
    drop(source);
    assert!(weak.upgrade().is_some());
    assert_eq!(
        sample_camera_light(&corrected_lease, Some(&field), point).unwrap(),
        original * 2.0
    );
    drop(corrected_lease);
    assert!(weak.upgrade().is_none());
}

#[test]
fn synchronized_handoff_concurrent_readers_and_receiver_final_release() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<DevelopedSource>();
    let file = fixture::bayer_dng();
    let source = DevelopedSource::open(file.path(), Some(WB)).unwrap();
    let expected = codes(&source);
    let weak = Arc::downgrade(&source);
    let barrier = Arc::new(Barrier::new(3));
    let (send, receive) = std::sync::mpsc::channel::<Arc<DevelopedSource>>();
    let (finished, wait) = std::sync::mpsc::channel();
    let reader = source.clone();
    let reader_barrier = barrier.clone();
    let a = std::thread::spawn(move || {
        reader_barrier.wait();
        for _ in 0..8 {
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
        for _ in 0..8 {
            assert!(reference(&source, None, region()).is_ok());
        }
        let actual = codes(&source);
        wait.recv().unwrap();

        drop(source);
        actual
    });

    send.send(source).unwrap();
    drop(send);
    barrier.wait();
    assert_eq!(a.join().unwrap(), expected);
    assert_eq!(b.join().unwrap(), expected);
    assert!(weak.upgrade().is_none());
}
