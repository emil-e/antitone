use super::*;
use crate::geometry::SourceRegion;

fn positions(
    source: SourceRegion,
    area: SourceRegion,
    minimum_x: u32,
    budget: usize,
) -> Vec<[u32; 2]> {
    let columns = ((budget as f64 * f64::from(source.width) / f64::from(source.height)).sqrt()
        as usize)
        .max(1)
        .min(budget)
        .min(source.width as usize);
    let rows = (budget / columns).min(source.height as usize);
    (0..rows)
        .flat_map(|y| {
            (0..columns).map(move |x| {
                [
                    source.x + ((2 * x + 1) * source.width as usize / (2 * columns)) as u32,
                    source.y + ((2 * y + 1) * source.height as usize / (2 * rows)) as u32,
                ]
            })
        })
        .filter(|p| {
            p[0] >= minimum_x
                && area.contains_region(SourceRegion {
                    x: p[0],
                    y: p[1],
                    width: 1,
                    height: 1,
                })
        })
        .collect()
}

fn variable_field(a: SourceRegion) -> Arc<crate::flat_field::FlatField> {
    Arc::new(
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
    )
}

#[test]
fn cached_camera_sampling_preserves_original_correction_coordinates_and_omits_reference_exposure() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let mut scan = Scan::open(file.path(), None).unwrap();
    let a = scan.area;
    scan.set_flat_field(Some(variable_field(a)));
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
        .sample(area, 16, |p| p.x >= a.x + 20, || false)
        .unwrap()
        .unwrap();
    let expected_points = positions(a, area, a.x + 20, 16);
    assert_eq!(
        [actual.attempted, actual.rejected_floor],
        [expected_points.len(), 0]
    );
    assert_eq!(actual.pixels.len(), expected_points.len());
    let mut coordinate_control: f64 = 0.;
    for (actual, [x, y]) in actual.pixels.iter().zip(expected_points) {
        let expected = scan.pixel(x, y).unwrap();
        assert!((*actual - expected).abs().max_element() < 2e-6 * expected.abs().max_element());
        coordinate_control = coordinate_control.max(
            (expected - scan.pixel(x - 8, y - 8).unwrap())
                .abs()
                .max_element(),
        );
    }
    assert!(coordinate_control > 1e-3);
    assert!(
        sampler
            .sample(area, 16, |_| true, || true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn gathered_work_reads_only_selected_cached_centres_after_the_original_is_released() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = Scan::open(file.path(), None).unwrap();
    let weak = Arc::downgrade(scan.source());
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    drop(scan);
    assert!(weak.upgrade().is_none());
    let count = crate::roll_analysis::MAX_SAMPLES_PER_FRAME;
    let image = sampler.gathered(vec![0; count]).unwrap();
    let request = sampler
        .context
        .request(
            image.clone(),
            evaluation::Region {
                bounds: image.domain().finite().unwrap(),
                grid: image.domain().grid,
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
                assert!(pixels.iter().all(|p| p[3] == 0.));
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(delivered, count);
    assert_eq!(stats.source_cells, count as u64);
    assert!(stats.submissions <= 8, "{stats:?}");
}

#[test]
fn numerical_admission_checks_the_original_and_corrected_sample_that_is_actually_used() {
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
    )));
    let gate = SubmissionGate::default();
    let sampler = CameraSampler::new(&device, &renderer, (&queue, &gate), &scan).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let actual = sampler
        .sample(area, 16, |_| true, || false)
        .unwrap()
        .unwrap();
    let points = positions(a, area, area.x, 16);
    let expected: Vec<_> = points
        .iter()
        .map(|&[x, y]| scan.pixel(x, y).unwrap())
        .filter(|p| p.min_element() > scan.effective_floor())
        .collect();
    assert!(!expected.is_empty() && expected.len() < points.len());
    assert_eq!(actual.attempted, points.len());
    assert_eq!(actual.rejected_floor, points.len() - expected.len());
    assert_eq!(actual.pixels.len(), expected.len());
    for (actual, expected) in actual.pixels.iter().zip(expected) {
        assert!((*actual - expected).abs().max_element() < 2e-6 * expected.abs().max_element());
    }
}

#[test]
fn one_cached_payload_supports_changed_regions_and_correction_without_original_pixels() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let gate = SubmissionGate::default();
    for rgb in [[6000, 12000, 9000], [2; 3], [65535; 3]] {
        let file = crate::raw::fixture::bayer_dng_with_rgb(rgb);
        let scan = Scan::open(file.path(), None).unwrap();
        let a = scan.area;
        let weak = Arc::downgrade(scan.source());
        let input = Arc::new(AnalysisInput::capture(&scan, || false).unwrap().unwrap());
        let bytes = input.retained_bytes();
        drop(scan);
        assert!(weak.upgrade().is_none());
        let mut reference = Scan::open(file.path(), None).unwrap();
        for field in [None, Some(variable_field(a))] {
            reference.set_flat_field(field.clone());
            let sampler =
                CameraSampler::retained(&device, &renderer, (&queue, &gate), input.clone(), field)
                    .unwrap();
            assert!(Arc::ptr_eq(&sampler.input, &input));
            assert_eq!(sampler.effective_floor(), reference.effective_floor());
            for minimum_x in [a.x, a.x + 12] {
                let actual = sampler
                    .sample(a, 16, |p| p.x >= minimum_x, || false)
                    .unwrap()
                    .unwrap();
                let expected: Vec<_> = positions(a, a, minimum_x, 16)
                    .iter()
                    .map(|&[x, y]| reference.pixel(x, y).unwrap())
                    .filter(|p| p.min_element() > reference.effective_floor())
                    .collect();
                assert_eq!(actual.pixels.len(), expected.len());
                for (actual, expected) in actual.pixels.iter().zip(expected) {
                    assert!(
                        (*actual - expected).abs().max_element()
                            < 2e-6 * expected.abs().max_element()
                    );
                }
            }
            assert_eq!(input.retained_bytes(), bytes);
        }
        let weak = Arc::downgrade(&input);
        drop(input);
        assert!(
            weak.upgrade().is_none(),
            "completed GPU queries must not pin the cached payload"
        );
    }
}

#[test]
fn native_white_remains_usable_evidence() {
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
