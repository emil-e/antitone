use super::*;
use crate::processing::{InputPipeline, InputSettings};
use crate::raw::{RawFile, fixture};

#[test]
fn differing_preview_defaults_share_native_storage_and_preserve_existing_interpretations() {
    use crate::exposure_reference::{ReferenceExposure, StartingExposure};
    let file = fixture::bayer_dng();
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let first = scan
        .source()
        .interpreted(&ReferenceExposure::Matched(StartingExposure {
            ev: 1.,
            samples: 1024,
            spread: 0.,
        }));
    let second = first.interpreted(&ReferenceExposure::Matched(StartingExposure {
        ev: 2.,
        samples: 1024,
        spread: 0.,
    }));
    assert!(Arc::ptr_eq(&first.raw, &second.raw));
    assert_eq!(
        first.positive_matrix(&InputSettings::Digital).unwrap(),
        first.camera_to_ap1() * 2.
    );
    assert_eq!(
        second.positive_matrix(&InputSettings::Digital).unwrap(),
        second.camera_to_ap1() * 4.
    );
}

#[test]
fn source_reads_restore_green_scale_without_rewriting_native_rgb16() {
    let file = fixture::bayer_dng();
    for (wb, gain) in [
        ([1.; 4], 1.),
        ([1e-6; 4], 1.),
        ([4., 1., 2., 1.], 4.),
        ([8., 2., 4., 2.], 4.),
        ([0.5, 1., 0.25, 1.], 1.),
        ([1., 1., 0.5, 3.5], 3.5),
    ] {
        let scan =
            Scan::from_raw(RawFile::open_uncorrected(file.path()).unwrap(), Some(wb)).unwrap();
        let native = scan.source.raw.camera_image().unwrap();
        let codes = native.pixel_codes(32, 32).unwrap();
        let view = scan.source.camera_image().unwrap();
        let samples = view.pixel_samples(32, 32).unwrap();
        for c in 0..3 {
            assert_eq!(samples[c], f32::from(codes[c]) * gain as f32);
        }
        assert_eq!(view.is_continuous(), gain != 1.);
        assert_eq!(
            scan.source.input_floor(),
            crate::processing::CAMERA_INPUT_FLOOR * gain
        );
        assert_eq!(
            scan.source
                .raw
                .camera_image()
                .unwrap()
                .pixel_codes(32, 32)
                .unwrap(),
            codes
        );
    }
}

#[test]
fn digital_neutral_reference_reaches_ap1_middle_gray_at_zero_user_exposure() {
    // Black=0, white=65535; the declared WB makes all three camera channels
    // 11796 codes, within 0.3 code of 18% white. LibRaw's max-WB scale is 1/4.
    let file = fixture::bayer_dng_with_rgb([2949, 11796, 5898]);
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([4., 1., 2., 1.]),
    )
    .unwrap();
    let pixel = scan.pixel(32, 32).unwrap();
    assert!(
        (pixel - DVec3::splat(0.18)).abs().max_element() < 2e-5,
        "{pixel:?}"
    );
    let pipeline = InputPipeline::new(
        InputSettings::Digital,
        Default::default(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let ap1 = pipeline.apply(pixel).unwrap().rgb;
    assert!(
        (ap1 - DVec3::splat(0.18)).abs().max_element() < 5e-5,
        "{ap1:?}"
    );
    let proxy = CameraProxy::from_scan(&scan).unwrap();
    let proxy = proxy.pixels[(32 + 32 * proxy.width) as usize];
    for value in &proxy[..3] {
        assert!((f64::from(*value) / 65535. - 0.18).abs() < 2e-5);
    }
}

#[test]
fn normalized_source_samples_preserve_above_one_headroom() {
    let file = fixture::bayer_dng_with_rgb([64000; 3]);
    let scan = Scan::from_raw(
        RawFile::open_uncorrected(file.path()).unwrap(),
        Some([4., 1., 2., 1.]),
    )
    .unwrap();
    let sample = scan.pixel(32, 32).unwrap();
    assert!((sample.x - 4. * 64000. / 65535.).abs() < 1e-4);
    assert!(sample.x > 1. && sample.z > 1.);
    assert!(
        scan.source
            .camera_image()
            .unwrap()
            .pixel_codes(32, 32)
            .is_err()
    );
    assert_eq!(
        scan.source
            .raw
            .camera_image()
            .unwrap()
            .pixel_codes(32, 32)
            .unwrap()[0],
        64000
    );
}

#[test]
fn source_normalization_scales_continuous_lens_samples_and_their_precision_together() {
    use crate::raw::sony_fixture as sony;
    let order = sony::Order::Little;
    let file = sony::source_with_as_shot(order, sony::profile_tags(order), [4, 1, 2]);
    let scan = Scan::open(file.path(), None).unwrap();
    assert!(scan.source.metadata().lens_identity().unwrap().is_some());
    let native = scan.source.raw.camera_image().unwrap();
    assert!(native.is_continuous());
    let original = native.pixel_samples(32, 32).unwrap();
    let normalized = scan
        .source
        .camera_image()
        .unwrap()
        .pixel_samples(32, 32)
        .unwrap();
    assert_eq!(normalized, original.map(|value| value * 4.));
    assert_eq!(
        scan.source.input_floor(),
        scan.source.metadata().input_floor().unwrap() * 4.
    );
}

#[cfg(feature = "dng-sdk")]
#[test]
fn sdk_developed_samples_are_not_rescaled_by_the_libraw_gain() {
    let file = fixture::bayer_dng_with_rgb([2949, 11796, 5898]);
    let scan = Scan::open(file.path(), Some([4., 1., 2., 1.])).unwrap();
    let native = scan.source.raw.camera_image().unwrap();
    let view = scan.source.camera_image().unwrap();
    assert_eq!(
        view.pixel_samples(32, 32).unwrap(),
        native.pixel_samples(32, 32).unwrap()
    );
    assert_eq!(
        scan.source.input_floor(),
        scan.source.metadata().input_floor().unwrap()
    );
    let ap1 = InputPipeline::ungraded(InputSettings::Digital, scan.camera_to_ap1)
        .unwrap()
        .apply(scan.pixel(32, 32).unwrap())
        .unwrap()
        .rgb;
    assert!(
        (ap1 - DVec3::splat(0.18)).abs().max_element() < 5e-5,
        "{ap1:?}"
    );
}
