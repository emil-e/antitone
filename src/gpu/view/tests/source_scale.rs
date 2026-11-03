use super::*;

#[test]
fn digital_original_and_navigation_positive_share_restored_neutral_exposure_and_headroom() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        for (rgb, ev) in [
            ([2949, 11796, 5898], 0.),
            ([64000; 3], 0.),
            ([2949, 11796, 5898], -14.),
        ] {
            let file = crate::raw::fixture::bayer_dng_with_rgb(rgb);
            let mut scan = Scan::from_raw(
                crate::raw::RawFile::open_uncorrected(file.path()).unwrap(),
                Some([4., 1., 2., 1.]),
            )
            .unwrap();
            if ev != 0. {
                scan.reference_exposure =
                    Some(crate::exposure_reference::ReferenceExposure::Matched(
                        crate::exposure_reference::StartingExposure {
                            ev,
                            samples: 1024,
                            spread: 0.,
                        },
                    ));
            }
            let gain = ev.exp2();
            let expected = scan.camera_to_ap1
                * gain
                * DVec3::new(
                    4. * f64::from(rgb[0]),
                    f64::from(rgb[1]),
                    2. * f64::from(rgb[2]),
                )
                / 65535.;
            if rgb[0] == 64000 {
                assert!(expected.max_element() > 1.);
            }
            let spec = SourceRequest::new(
                frame(&scan),
                InputSettings::Digital,
                scan.positive_matrix(&InputSettings::Digital).unwrap(),
                None,
            )
            .unwrap();
            let original = ViewSource::acquire(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                spec.clone(),
                || false,
            )
            .unwrap()
            .unwrap();
            let proxy = crate::scan::CameraProxy::from_scan(&scan).unwrap();
            let input = ProcessingInput::new(
                &device.limits(),
                InputSettings::Digital,
                RollSettings {
                    exposure: 1.,
                    ..Default::default()
                },
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
                scan.positive_matrix(&InputSettings::Digital).unwrap(),
                scan.area,
            )
            .unwrap();
            let (navigation, processed) = ViewSource::camera_preview(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &proxy,
                None,
                spec,
                &input,
            )
            .unwrap();
            assert!(
                processed
                    .definition
                    .region_for(
                        &navigation.definition,
                        processed.definition.domain().finite().unwrap()
                    )
                    .unwrap()
                    .is_some(),
                "processed pixels retain their actual native calculation"
            );
            for (pixels, expected) in [
                (&original.pixels, expected),
                (&navigation.pixels, expected),
                (&processed.pixels, expected * 2.),
            ] {
                let values = read(&device, &queue, pixels);
                let pixel = values[32 + 32 * 64];
                for channel in 0..3 {
                    assert!(
                        (f64::from(pixel[channel]) - expected[channel]).abs() < 5e-5 * gain,
                        "{pixel:?} vs {expected:?}"
                    );
                }
            }
        }
    });
}
