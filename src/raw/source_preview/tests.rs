use super::*;

#[test]
fn viewing_white_operates_in_camera_channels_not_working_rgb() {
    let basis = DMat3::from_cols(
        DVec3::new(0.8, 0.1, -0.05),
        DVec3::new(0.2, 0.85, 0.1),
        DVec3::new(-0.1, 0.05, 0.95),
    );
    // These sensor values are neutral under the declared viewing gains [2,1,4].
    let sensor = DVec3::new(0.25, 0.5, 0.125);
    for fixed in [[1.; 4], [3., 2., 4., 2.]] {
        let developed = sensor * DVec3::from_array([fixed[0], fixed[1], fixed[2]]) / fixed[1];
        let view = native_transform(basis, fixed, [2., 1., 4., 1.]).unwrap();
        let actual = view * (basis * developed);
        let expected = basis * DVec3::splat(0.5);
        assert!((actual - expected).abs().max_element() < 1e-12);
    }
}

#[test]
fn native_viewing_white_matches_an_independent_balanced_development() {
    let file = crate::raw::fixture::bayer_dng();
    let desired = [2., 1., 0.5, 1.];
    let unity = crate::scan::Scan::from_raw(
        crate::raw::RawFile::open_uncorrected(file.path()).unwrap(),
        Some([1.; 4]),
    )
    .unwrap();
    let balanced = crate::scan::Scan::from_raw(
        crate::raw::RawFile::open_uncorrected(file.path()).unwrap(),
        Some(desired),
    )
    .unwrap();
    let mut metadata = unity.source().metadata();
    metadata.cam_mul = desired;
    let preview = metadata.source_preview([1.; 4]);
    let before = unity.pixel(32, 32).unwrap();
    let actual = preview.transform(SourceWhiteBalance::AsShot) * (unity.camera_to_ap1 * before);
    let expected = balanced.camera_to_ap1 * balanced.pixel(32, 32).unwrap();
    assert!((actual - expected).abs().max_element() < 4. / 65535.);
    assert_eq!(unity.pixel(32, 32).unwrap(), before);
}

#[cfg(feature = "dng-sdk")]
#[test]
fn sdk_viewing_white_uses_profile_adaptation_without_repeating_development_gains() {
    let file = crate::raw::fixture::bayer_dng();
    let mut bytes = std::fs::read(file.path()).unwrap();
    let count = u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as usize;
    let entry = (0..count)
        .map(|i| 10 + i * 12)
        .find(|offset| u16::from_le_bytes(bytes[*offset..*offset + 2].try_into().unwrap()) == 50728)
        .unwrap();
    let offset = u32::from_le_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
    bytes[offset + 4..offset + 8].copy_from_slice(&2u32.to_le_bytes());
    bytes[offset + 16..offset + 20].copy_from_slice(&2u32.to_le_bytes());
    std::fs::write(file.path(), &bytes).unwrap();
    let mut unity = crate::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
    let mut balanced = crate::scan::Scan::open(file.path(), None).unwrap();
    assert_ne!(balanced.fixed_wb, [1.; 4]);
    let preview = unity.source().metadata().source_preview(unity.fixed_wb);
    let codes = |scan: &mut crate::scan::Scan| {
        let pixel = scan.tile_region(scan.area, 32, 32, 1, 1).unwrap()[0];
        DVec3::new(
            f64::from(pixel[0]),
            f64::from(pixel[1]),
            f64::from(pixel[2]),
        ) / 65535.
    };
    let before = codes(&mut unity);
    let actual = preview.transform(SourceWhiteBalance::AsShot) * (unity.camera_to_ap1 * before);
    let balanced_codes = codes(&mut balanced);
    let expected = balanced.camera_to_ap1 * balanced_codes;
    assert!(
        (actual - expected).abs().max_element() < 1e-6,
        "{actual:?} != {expected:?}"
    );
    assert_eq!(codes(&mut unity), before);
    let daylight = preview.transform(SourceWhiteBalance::Daylight) * (unity.camera_to_ap1 * before);
    assert!((daylight - actual).abs().max_element() > 0.01);
}

#[test]
fn missing_or_unusable_viewing_metadata_falls_back_without_mutating_it() {
    let daylight = DMat3::from_diagonal(DVec3::new(2., 1., 1.5));
    let preview = SourcePreview {
        daylight: Some(daylight),
        as_shot: None,
    };
    assert_eq!(preview.transform(SourceWhiteBalance::AsShot), daylight);
    assert_eq!(
        SourcePreview::default().transform(SourceWhiteBalance::Daylight),
        DMat3::IDENTITY
    );
    for desired in [
        [0.; 4],
        [f64::NAN, 1., 2., 1.],
        [1., -1., 2., 1.],
        [1e300, 1., 2., 1.],
    ] {
        assert!(native_transform(DMat3::IDENTITY, [1.; 4], desired).is_none());
    }
    assert!(native_transform(DMat3::IDENTITY, [1., 0., 1., 1.], [2., 1., 1., 1.]).is_none());
}
