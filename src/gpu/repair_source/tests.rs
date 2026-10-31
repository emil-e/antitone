use super::*;
use glam::DVec3;

#[test]
fn repair_inputs_share_corrected_signed_hdr_and_negative_values_across_blocks_and_cache_policies() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let gate = SubmissionGate::default();
    let no_cache =
        evaluation::Context::new(&device, evaluation::CachePolicy { generated_bytes: 0 });
    let file = crate::raw::fixture::bayer_dng_with_camera(
        [260, 1000, 600],
        glam::DMat3::IDENTITY.to_cols_array(),
    );
    let mut scan = Scan::open(file.path(), None).unwrap();
    let area = scan.area;
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![
                DVec3::new(0.9, 1.1, 0.8),
                DVec3::new(1.2, 0.9, 1.1),
                DVec3::new(1., 1.2, 0.9),
                DVec3::new(0.8, 1., 1.2),
            ],
        )
        .unwrap(),
    );
    scan.set_flat_field(Some(field)).unwrap();
    let region = SourceRegion::from([area.x + 11, area.y + 13, 17, 9]);
    let needed: Vec<_> = (0..region.width * region.height)
        .map(|index| index % 3 != 0)
        .collect();

    for (input, ev) in [
        (InputSettings::Digital, 0.),
        (InputSettings::Digital, 8.),
        (
            crate::processing::Calibration {
                dmin: DVec3::splat(0.8),
                dmax: DVec3::splat(0.02),
                input_floor: scan.source().input_floor(),
            }
            .into(),
            0.,
        ),
    ] {
        scan.reference_exposure = Some(crate::exposure_reference::ReferenceExposure::Matched(
            crate::exposure_reference::StartingExposure {
                ev,
                samples: 1024,
                spread: 0.,
            },
        ));
        let reference = crate::scan::ImageSource::new(&scan, input).unwrap();
        let mut uncorrected_scan =
            Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
        uncorrected_scan.set_flat_field(None).unwrap();
        let uncorrected = crate::scan::ImageSource::new(&uncorrected_scan, input).unwrap();
        let mut source = Source::new(&device, &renderer, (&queue, &gate), &scan, input).unwrap();
        source.budget.tile = [3, 2];
        let pixels = source
            .read(region, Some(&needed), &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(pixels.len(), needed.len());
        let mut correction_difference: f64 = 0.;
        let mut signed = false;
        let mut hdr = false;

        for (index, pixel) in pixels.iter().enumerate() {
            if !needed[index] {
                assert_eq!(*pixel, [0.; 3]);
                continue;
            }
            let point = [
                region.x + index as u32 % region.width,
                region.y + index as u32 / region.width,
            ];
            let expected = reference.sample(point).unwrap();
            correction_difference = correction_difference.max(
                (expected - uncorrected.sample(point).unwrap())
                    .abs()
                    .max_element(),
            );
            signed |= expected.min_element() < 0.;
            hdr |= expected.max_element() > 1.;
            for channel in 0..3 {
                assert!(
                    (f64::from(pixel[channel]) - expected[channel]).abs()
                        < 2e-5 * expected.abs().max_element().max(1.),
                    "{input:?} {point:?}: {pixel:?} != {expected:?}"
                );
            }
        }
        assert!(
            correction_difference > 1e-4,
            "the fixture must distinguish correction omission"
        );
        if input == InputSettings::Digital {
            assert!(
                signed,
                "the camera profile must exercise signed working light"
            );
            if ev != 0. {
                assert!(hdr, "the captured exposure must exercise HDR light");
            }
        }

        source.budget.tile = [5, 3];
        assert_eq!(
            source
                .read(region, Some(&needed), &|| false)
                .unwrap()
                .unwrap(),
            pixels
        );
        source.context = &no_cache;
        assert_eq!(
            source
                .read(region, Some(&needed), &|| false)
                .unwrap()
                .unwrap(),
            pixels
        );
        assert!(source.read(region, None, &|| true).unwrap().is_none());
        assert_eq!(
            source
                .read(region, Some(&needed), &|| false)
                .unwrap()
                .unwrap(),
            pixels
        );
    }
    assert_eq!(no_cache.retained_bytes(), 0);
}

#[test]
fn repair_inputs_ignore_unrepresentable_masked_values_but_reject_required_ones() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([0; 3]);
    let scan = Scan::open(file.path(), None).unwrap();
    let mut input = InputSettings::negative(crate::processing::Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
        input_floor: scan.source().input_floor(),
    });
    if let InputSettings::Negative { gamma, .. } = &mut input {
        *gamma = 0.1;
    }
    let region = SourceRegion::from([scan.area.x + 16, scan.area.y + 16, 2, 2]);
    let scalar = crate::scan::ImageSource::new(&scan, input)
        .unwrap()
        .sample([region.x, region.y])
        .unwrap();
    assert!(
        scalar.is_finite() && !scalar.as_vec3().is_finite(),
        "the fixture must distinguish f64 evaluation from f32 repair storage"
    );
    let gate = SubmissionGate::default();
    let mut source = Source::new(&device, &renderer, (&queue, &gate), &scan, input).unwrap();
    assert_eq!(
        source
            .read(region, Some(&[false; 4]), &|| false)
            .unwrap()
            .unwrap(),
        vec![[0.; 3]; 4]
    );
    assert!(matches!(
        source.read(region, None, &|| false),
        Err(Error::NumericalOverflow)
    ));
    assert_eq!(
        source
            .read(region, Some(&[false; 4]), &|| false)
            .unwrap()
            .unwrap(),
        vec![[0.; 3]; 4]
    );
}

#[test]
fn repair_inputs_accept_developed_zero_and_native_white() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([0, 65535, 600]);
    let scan = Scan::open(file.path(), None).unwrap();
    let region = SourceRegion::from([scan.area.x + 16, scan.area.y + 16, 8, 8]);
    let camera = scan.pixel(region.x, region.y).unwrap();
    assert_eq!(camera.x, 0.);
    assert_eq!(camera.y, 1.);
    let gate = SubmissionGate::default();
    let mut source = Source::new(
        &device,
        &renderer,
        (&queue, &gate),
        &scan,
        InputSettings::Digital,
    )
    .unwrap();
    let pixels = source.read(region, None, &|| false).unwrap().unwrap();
    assert_eq!(pixels.len(), 64);
    assert!(pixels.iter().flatten().all(|value| value.is_finite()));
}
