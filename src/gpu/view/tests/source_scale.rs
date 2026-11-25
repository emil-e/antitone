use super::*;
use crate::gpu::evaluation as e;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::photographic::graph::view_source::SourceRequest;

#[test]
fn digital_original_and_navigation_positive_share_restored_neutral_exposure_and_headroom() {
    pollster::block_on(async {
        let (device, queue, renderer, _) = context().await;
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
            let input = AuthoredPhotograph::new(
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
            let capture = crate::photographic::graph::request::PhotographRequest::Fixture(input);
            let original = capture
                .original_neutral(
                    crate::gpu::source::camera_source(&scan).unwrap(),
                    None,
                    scan.camera_to_ap1,
                )
                .unwrap();
            let proxy = crate::scan::CameraProxy::from_scan(&scan).unwrap();
            let camera = crate::gpu::source::ProcessingSource::new_continuous(
                &device,
                &proxy.pixels,
                proxy.matrix,
                Some(
                    crate::gpu::source::SourceGeometry::new(
                        [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                        proxy.dimensions(),
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
            let mut presentation = frame(&scan);
            presentation.output = proxy.dimensions();
            let spec = SourceRequest::for_processing(
                presentation,
                &capture.processing(&device.limits()).unwrap(),
            )
            .unwrap()
            .with_sampling(crate::acquisition::Sampling::CameraCells {
                region: scan.area,
                size: proxy.dimensions(),
            })
            .unwrap();
            let baseline = capture.sampled_prefix(&camera, None, &spec).unwrap();
            let (processed, _) = capture
                .sampled_view(&device.limits(), &camera, None, spec)
                .unwrap();
            assert!(
                processed
                    .region_for(&baseline, processed.domain().finite().unwrap())
                    .unwrap()
                    .is_some()
            );
            for (image, expected) in [
                (original, expected),
                (baseline, expected),
                (processed, expected * 2.),
            ] {
                let domain = image.domain();
                let region = domain.finite().unwrap();
                let evaluation = renderer
                    .evaluation()
                    .request(
                        image,
                        e::Region {
                            bounds: region,
                            grid: domain.grid,
                        },
                        e::Tiling { tile: [64, 64] },
                    )
                    .unwrap()
                    .without_retention();
                let context = renderer.evaluation();
                let pixels = context
                    .materialize_many(&queue, &context.materializations(), vec![evaluation])
                    .unwrap()
                    .remove(0)
                    .await
                    .unwrap();
                let values = read(&device, &queue, pixels.as_buffer().unwrap());
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
