use super::*;
use crate::geometry::SourceRegion;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn large_asymmetric_picture_grid_keeps_rounded_dimensions_and_original_pixel_centres() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 1024,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Scan::open(file.path(), None).unwrap();
    let area = SourceRegion::from([scan.area.x + 12, scan.area.y + 20, 901, 641]);
    assert!(scan.area.contains_region(area));
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let raster = sampler
        .spatial(area, |f| area.contains_region(f), || false)
        .unwrap()
        .unwrap();
    assert_eq!(raster.size(), [512, 364]);
    assert_eq!(raster.camera_samples().len(), 512 * 364);
    assert!(!raster.eligibility()[0]);
    for [x, y] in [[3usize, 3usize], [250, 183], [508, 360]] {
        let sx = area.x + ((2 * x + 1) * area.width as usize / (2 * 512)) as u32;
        let sy = area.y + ((2 * y + 1) * area.height as usize / (2 * 364)) as u32;
        let reference = scan.pixel(sx, sy).unwrap();
        assert!(raster.eligibility()[y * 512 + x]);
        assert!(
            (raster.camera_samples()[y * 512 + x] - reference)
                .abs()
                .max_element()
                < 2e-6 * reference.abs().max_element()
        );
    }
}

#[test]
fn spatial_camera_sites_keep_physical_correction_support_and_omit_reference_exposure() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let mut scan = Scan::open(file.path(), None).unwrap();
    let weak = Arc::downgrade(scan.source());
    let a = scan.area;
    scan.set_flat_field(Some(Arc::new(
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
    )))
    .unwrap();
    scan.reference_exposure = Some(crate::exposure_reference::ReferenceExposure::Matched(
        crate::exposure_reference::StartingExposure {
            ev: 8.,
            samples: 1024,
            spread: 0.,
        },
    ));
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let inside = |f: SourceRegion| area.contains_region(f) && f.x >= a.x + 20;
    let raster = sampler.spatial(area, inside, || false).unwrap().unwrap();
    assert_eq!(raster.size(), [48, 48]);
    let mut kept = 0;
    let mut coordinate_control: f64 = 0.;
    for y in 0..48 {
        for x in 0..48 {
            let (sx, sy) = (a.x + 8 + x, a.y + 8 + y);
            let f = SourceRegion::from([sx - 4, sy - 4, 8, 8]);
            let expected = inside(f);
            let i = (y * 48 + x) as usize;
            assert_eq!(raster.eligibility()[i], expected);
            if expected {
                kept += 1;
                let reference = scan.pixel(sx, sy).unwrap();
                assert!(
                    (raster.camera_samples()[i] - reference).abs().max_element()
                        < 2e-6 * reference.abs().max_element()
                );
                coordinate_control = coordinate_control.max(
                    (reference - scan.pixel(sx - 8, sy - 8).unwrap())
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
fn corrected_neighbours_can_reject_an_otherwise_usable_spatial_centre() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([2; 3]);
    let mut scan = Scan::open(file.path(), None).unwrap();
    let a = scan.area;
    scan.set_flat_field(Some(Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![
                DVec3::splat(0.4),
                DVec3::splat(2.),
                DVec3::splat(0.4),
                DVec3::splat(2.),
            ],
        )
        .unwrap(),
    )))
    .unwrap();
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let raster = sampler
        .spatial(area, |f| area.contains_region(f), || false)
        .unwrap()
        .unwrap();
    assert!(scan.pixel(a.x + 26, a.y + 26).unwrap().min_element() > scan.effective_floor());
    assert!(scan.pixel(a.x + 22, a.y + 22).unwrap().min_element() <= scan.effective_floor());
    assert!(
        !raster.eligibility()[18 * 48 + 18],
        "full corrected footprint, not merely its centre"
    );
    assert!(
        raster.eligibility()[18 * 48 + 30],
        "contrasting valid footprint must remain usable"
    );
}

#[test]
fn uncorrected_spatial_capture_accepts_native_white_and_releases_the_original_lease() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([65535; 3]);
    let scan = Scan::open(file.path(), None).unwrap();
    let weak = Arc::downgrade(scan.source());
    let area = SourceRegion::from([scan.area.x + 8, scan.area.y + 8, 48, 48]);
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
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
