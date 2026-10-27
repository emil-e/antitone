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
            let spec = ViewSourceSpec::new(
                1,
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
            let navigation = ViewSource::from_camera_proxy(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &proxy,
                None,
                spec,
            )
            .unwrap();
            for source in [original, navigation] {
                let values = read(&device, &queue, &source.pixels);
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
