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
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let a = scan.area();
    let scan_field = Some(variable_field(a));
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
    let sampler =
        CameraSampler::new(renderer.evaluation(), &queue, &scan, scan_field.clone()).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let actual = sampler
        .sample(area, 16, |p| p.x >= a.x + 20, || false)
        .unwrap()
        .unwrap();
    let expected_points = positions(a, area, a.x + 20, 16);
    assert_eq!(actual.pixels.len(), expected_points.len());
    let mut coordinate_control: f64 = 0.;
    for (actual, [x, y]) in actual.pixels.iter().zip(expected_points) {
        let expected =
            crate::flat_field::sample_camera_light(&scan, scan_field.as_deref(), [x, y]).unwrap();
        assert!((*actual - expected).abs().max_element() < 2e-6 * expected.abs().max_element());
        coordinate_control = coordinate_control.max(
            (expected
                - crate::flat_field::sample_camera_light(
                    &scan,
                    scan_field.as_deref(),
                    [x - 8, y - 8],
                )
                .unwrap())
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
fn roll_sampling_rejects_an_oversized_request_even_if_no_sites_would_pass() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let source = DevelopedSource::open(file.path(), None).unwrap();
    let sampler = CameraSampler::new(renderer.evaluation(), &queue, &source, None).unwrap();
    let area = source.area();
    let limit = crate::roll_analysis::MAX_SAMPLES_PER_FRAME;

    let selected = sampler
        .input
        .select(area, limit + 1, |_| true, &mut || false)
        .unwrap()
        .unwrap();
    assert!(!selected.is_empty());
    let empty = sampler
        .sample(area, limit, |_| false, || false)
        .unwrap()
        .unwrap();
    assert_eq!(empty.attempted, 0);
    assert!(matches!(
        sampler.sample(area, limit + 1, |_| false, || false),
        Err(Error::InvalidParameter(_))
    ));
}

#[test]
fn gathered_work_reads_only_selected_cached_centres_after_the_original_is_released() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let weak = Arc::downgrade(&scan);
    let sampler = CameraSampler::new(renderer.evaluation(), &queue, &scan, None).unwrap();
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
fn flat_corrected_camera_samples_are_kept_even_with_very_low_signal() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([2; 3]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let a = scan.area();
    let scan_field = Some(Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![
                DVec3::splat(0.1),
                DVec3::splat(2.),
                DVec3::splat(0.1),
                DVec3::splat(2.),
            ],
        )
        .unwrap(),
    ));
    let sampler =
        CameraSampler::new(renderer.evaluation(), &queue, &scan, scan_field.clone()).unwrap();
    let area = SourceRegion::from([a.x + 8, a.y + 8, 48, 48]);
    let actual = sampler
        .sample(area, 16, |_| true, || false)
        .unwrap()
        .unwrap();
    let points = positions(a, area, area.x, 16);
    let expected: Vec<_> = points
        .iter()
        .map(|&[x, y]| {
            crate::flat_field::sample_camera_light(&scan, scan_field.as_deref(), [x, y]).unwrap()
        })
        .collect();
    assert!(expected.iter().any(|p| p.min_element() < 1. / 65535.));
    assert!(expected.iter().any(|p| p.min_element() > 1. / 65535.));
    assert_eq!(actual.positions, points);
    assert_eq!(actual.pixels.len(), expected.len());
    for (actual, expected) in actual.pixels.iter().zip(expected) {
        assert!((*actual - expected).abs().max_element() < 2e-6 * expected.abs().max_element());
    }
}

#[test]
fn one_cached_payload_supports_changed_regions_and_correction_without_original_pixels() {
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    for rgb in [[6000, 12000, 9000], [2; 3], [65535; 3]] {
        let file = crate::raw::fixture::bayer_dng_with_rgb(rgb);
        let scan = DevelopedSource::open(file.path(), None).unwrap();
        let a = scan.area();
        let weak = Arc::downgrade(&scan);
        let input = Arc::new(AnalysisInput::capture(&scan, || false).unwrap().unwrap());
        let bytes = input.retained_bytes();
        drop(scan);
        assert!(weak.upgrade().is_none());
        let reference = DevelopedSource::open(file.path(), None).unwrap();
        for field in [None, Some(variable_field(a))] {
            let reference_field = field.clone();
            let sampler =
                CameraSampler::retained(renderer.evaluation(), &queue, input.clone(), field)
                    .unwrap();
            assert!(Arc::ptr_eq(&sampler.input, &input));
            for minimum_x in [a.x, a.x + 12] {
                let actual = sampler
                    .sample(a, 16, |p| p.x >= minimum_x, || false)
                    .unwrap()
                    .unwrap();
                let expected: Vec<_> = positions(a, a, minimum_x, 16)
                    .iter()
                    .map(|&[x, y]| {
                        crate::flat_field::sample_camera_light(
                            &reference,
                            reference_field.as_deref(),
                            [x, y],
                        )
                        .unwrap()
                    })
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
    let (_device, queue, renderer) = crate::photographic::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([65535; 3]);
    let scan = DevelopedSource::open(file.path(), None).unwrap();
    let sampler = CameraSampler::new(renderer.evaluation(), &queue, &scan, None).unwrap();
    let actual = sampler
        .sample(scan.area(), 1, |_| true, || false)
        .unwrap()
        .unwrap();
    assert_eq!(actual.pixels, vec![DVec3::ONE]);
}
