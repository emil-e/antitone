use super::*;
use crate::geometry::SourceRegion;

#[test]
fn camera_sampling_keeps_the_original_grid_correction_and_deliberate_exposure_omission() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let mut scan = Scan::open(file.path(), None).unwrap();
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
    let actual = sampler
        .sample(area, 16, |f| f.x >= a.x + 20, || false)
        .unwrap()
        .unwrap();
    assert_eq!([actual.attempted, actual.rejected_floor], [12, 0]);
    let expected_points: Vec<_> = [14, 26, 38, 50]
        .into_iter()
        .flat_map(|y| [26, 38, 50].map(move |x| [a.x + x, a.y + y]))
        .collect();
    assert_eq!(actual.pixels.len(), expected_points.len());
    let mut origin_control: f64 = 0.;
    for (actual, [x, y]) in actual.pixels.iter().zip(expected_points) {
        let expected = scan.pixel(x, y).unwrap();
        assert!((*actual - expected).abs().max_element() / expected.abs().max_element() < 2e-6);
        origin_control = origin_control.max(
            (expected - scan.pixel(x - 8, y - 8).unwrap())
                .abs()
                .max_element(),
        );
    }
    assert!(
        origin_control > 1e-3,
        "the field must distinguish original positions from rebased indices"
    );
    assert!(
        sampler
            .sample(area, 16, |_| true, || true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn gathered_camera_work_is_bounded_by_selected_cells_not_their_enclosing_raster() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = Scan::open(file.path(), None).unwrap();
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let count = crate::roll_analysis::MAX_SAMPLES_PER_FRAME;
    let origin = [scan.area.x + 8, scan.area.y + 8];
    let image = sampler.gathered(vec![origin; count]).unwrap();
    let request = sampler
        .context
        .request(
            image.clone(),
            evaluation::Region {
                bounds: image.domain.bounds,
                grid: image.domain.grid,
            },
            BUDGET,
        )
        .unwrap();
    let mut delivered = 0;
    let stats = sampler
        .context
        .read(
            &queue,
            &gate,
            &request,
            || false,
            |_, pixels| {
                delivered += pixels.len();
                assert!(pixels.iter().all(|pixel| pixel[3] == 0.));

                Ok(())
            },
        )
        .unwrap()
        .unwrap();

    // Repeated physical footprints exercise the maximum gathered batch on a small native
    // fixture. Work is exactly the selected 8x8 cells, with bounded aggregate submissions.
    assert_eq!(delivered, count * 64);
    assert_eq!(stats.source_cells, (count * 64) as u64);
    assert!(stats.submissions <= 8, "{stats:?}");
}

#[test]
fn camera_sampling_requires_every_cell_of_the_selected_capture_footprint() {
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
    assert!(scan.pixel(a.x + 26, a.y + 26).unwrap().min_element() > scan.effective_floor());
    assert!(
        scan.pixel(a.x + 22, a.y + 22).unwrap().min_element() <= scan.effective_floor(),
        "the centre alone must not establish eligibility"
    );
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let reference = scan.analysis_samples(area, 16, || false).unwrap().unwrap();
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let actual = sampler
        .sample(area, 16, |_| true, || false)
        .unwrap()
        .unwrap();
    assert_eq!([actual.attempted, actual.rejected_floor], [16, 8]);
    assert_eq!(actual.pixels.len(), 8);
    assert_eq!(actual.pixels.len(), reference.pixels.len());
    for (actual, expected) in actual.pixels.iter().zip(&reference.pixels) {
        assert!((*actual - *expected).abs().max_element() < 2e-6 * expected.abs().max_element());
    }
}

#[test]
fn camera_sampling_accepts_native_white_without_a_capture_or_white_value_veto() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([65535; 3]);
    let scan = Scan::open(file.path(), None).unwrap();
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let actual = sampler
        .sample(scan.area, 1, |_| true, || false)
        .unwrap()
        .unwrap();
    assert_eq!(actual.attempted, 1);
    assert_eq!(actual.pixels, vec![DVec3::ONE]);
    assert_eq!(actual.rejected_floor, 0);
}
