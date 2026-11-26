use super::*;
use crate::geometry::SourceRegion;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn large_asymmetric_picture_uses_a_strided_view_of_the_same_cached_source_grid() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 1024,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let area = SourceRegion::from([scan.area().x + 12, scan.area().y + 20, 901, 641]);
    assert!(scan.area().contains_region(area));
    let sampler = CameraSampler::new(renderer.evaluation(), &queue, &scan, None).unwrap();
    let raster = sampler
        .spatial(area, |f| area.contains_region(f), || false)
        .unwrap()
        .unwrap();
    assert_eq!(raster.size(), [451, 321]);
    assert_eq!(raster.camera_samples().len(), 451 * 321);
    assert!(raster.eligibility()[0]);
    for [x, y] in [[0usize, 0usize], [250, 183], [450, 320]] {
        let sx = area.x + 2 * x as u32;
        let sy = area.y + 2 * y as u32;
        let reference = scan.pixel(sx, sy).unwrap();
        assert!(raster.eligibility()[y * 451 + x]);
        assert!(
            (raster.camera_samples()[y * 451 + x] - reference)
                .abs()
                .max_element()
                < 2e-6 * reference.abs().max_element()
        );
    }
}

#[test]
fn spatial_camera_sites_keep_physical_correction_support_and_omit_reference_exposure() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let weak = Arc::downgrade(&scan);
    let a = scan.area();
    let scan_field = Some(Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![
                DVec3::new(0.8, 1.2, 0.9),
                DVec3::new(1.8, 0.9, 1.4),
                DVec3::new(1.1, 1.6, 1.3),
                DVec3::new(1.5, 1.3, 0.8),
            ],
        )
        .unwrap(),
    ));
    let positive = crate::scan::ImageSource::new(
        &scan,
        crate::processing::InputSettings::Digital,
        Some(8.),
        scan_field.clone(),
    )
    .unwrap();
    let point = [a.x + 24, a.y + 24];
    let unscaled =
        crate::flat_field::sample_camera_light(&scan, scan_field.as_deref(), point).unwrap();
    assert!(
        positive.sample(point).unwrap().abs().max_element()
            > 100. * (scan.camera_to_ap1() * unscaled).abs().max_element(),
        "a Digital baseline would substantially change these camera samples",
    );
    let gate = queue.clone();
    let sampler =
        CameraSampler::new(renderer.evaluation(), &queue, &scan, scan_field.clone()).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let inside = |f: SourceRegion| area.contains_region(f) && f.x >= a.x + 20;
    let raster = sampler.spatial(area, inside, || false).unwrap().unwrap();
    assert_eq!(raster.size(), [48, 48]);
    let mut kept = 0;
    let mut coordinate_control: f64 = 0.;
    for y in 0..48 {
        for x in 0..48 {
            let (sx, sy) = (a.x + 8 + x, a.y + 8 + y);
            let f = SourceRegion::from([sx, sy, 1, 1]);
            let expected = inside(f);
            let i = (y * 48 + x) as usize;
            assert_eq!(raster.eligibility()[i], expected);
            if expected {
                kept += 1;
                let reference =
                    crate::flat_field::sample_camera_light(&scan, scan_field.as_deref(), [sx, sy])
                        .unwrap();
                assert!(
                    (raster.camera_samples()[i] - reference).abs().max_element()
                        < 2e-6 * reference.abs().max_element()
                );
                coordinate_control = coordinate_control.max(
                    (reference
                        - crate::flat_field::sample_camera_light(
                            &scan,
                            scan_field.as_deref(),
                            [sx - 8, sy - 8],
                        )
                        .unwrap())
                    .abs()
                    .max_element(),
                );
            }
        }
    }
    assert!(kept > 0 && kept < 48 * 48);
    assert!(coordinate_control > 1e-3);
    assert!(sampler.spatial(area, inside, || true).unwrap().is_none());
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = stopped.clone();
    gate.observe_submissions(Arc::new(move || {
        signal.store(true, Ordering::SeqCst);
    }));
    assert!(
        sampler
            .spatial(area, inside, || stopped.load(Ordering::SeqCst))
            .unwrap()
            .is_none()
    );
    assert!(
        stopped.load(Ordering::SeqCst),
        "cancellation must reach an actual GPU submission"
    );
    drop(sampler);
    drop(scan);
    assert!(
        weak.upgrade().is_none(),
        "corrected capture must release the original lease"
    );
}

#[test]
fn corrected_spatial_samples_retain_low_light_inside_support() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([2; 3]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let a = scan.area();
    let scan_field = Some(Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![DVec3::splat(0.1), DVec3::ONE, DVec3::splat(0.1), DVec3::ONE],
        )
        .unwrap(),
    ));
    let sampler =
        CameraSampler::new(renderer.evaluation(), &queue, &scan, scan_field.clone()).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let raster = sampler
        .spatial(area, |f| area.contains_region(f), || false)
        .unwrap()
        .unwrap();
    let low =
        crate::flat_field::sample_camera_light(&scan, scan_field.as_deref(), [a.x + 22, a.y + 22])
            .unwrap();
    assert!(low.min_element() < 1. / 65535.);
    let index = 14 * 48 + 14;
    assert!(raster.eligibility()[index]);
    assert!((raster.camera_samples()[index] - low).abs().max_element() < 1e-10);
}

#[test]
fn uncorrected_spatial_capture_accepts_native_white_and_releases_the_original_lease() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([65535; 3]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let weak = Arc::downgrade(&scan);
    let area = SourceRegion::from([scan.area().x + 8, scan.area().y + 8, 48, 48]);
    let sampler = CameraSampler::new(renderer.evaluation(), &queue, &scan, None).unwrap();
    let raster = sampler
        .spatial(area, |f| area.contains_region(f), || false)
        .unwrap()
        .unwrap();
    let admitted: Vec<_> = raster
        .camera_samples()
        .iter()
        .zip(raster.eligibility())
        .filter(|(_, ok)| **ok)
        .map(|(p, _)| *p)
        .collect();
    assert!(!admitted.is_empty());
    assert!(admitted.iter().all(|p| *p == DVec3::ONE));
    drop(sampler);
    drop(scan);
    assert!(
        weak.upgrade().is_none(),
        "bounded spatial evidence must not retain original RAW data"
    );
    assert_eq!(raster.size(), [48, 48]);
}
