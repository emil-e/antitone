use super::*;

use crate::preview_fixture as fixture;

fn native_rgb(pixels: &[[u8; 3]], color_space: i32) -> Preview {
    decode(crate::raw::EmbeddedThumbnail {
        data: pixels.iter().flatten().copied().collect(),
        rgb_dimensions: Some([pixels.len() as u32, 1]),
        flip: 0,
        color_space,
    })
    .unwrap()
    .unwrap()
}

#[test]
fn native_transfer_and_primaries_determine_luminance_without_display_clipping() {
    let pixels = [
        [0; 3],
        [255; 3],
        [8; 3],
        [128; 3],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
    ];
    let srgb = native_rgb(&pixels, 1);
    let adobe = native_rgb(&pixels, 2);
    // Analytic transfer values and D65 primary Y from the sRGB / Adobe RGB definitions.
    let expected_srgb = [
        0.,
        1.,
        0.0024282158683907,
        0.21586050011389916,
        0.2126,
        0.7152,
        0.0722,
    ];
    let expected_adobe = [
        0.,
        1.,
        0.000493837590395553,
        0.21963795326060322,
        0.29734497525053616,
        0.6273635662554659,
        0.0752914584939979,
    ];
    for (preview, expected) in [(&srgb, expected_srgb), (&adobe, expected_adobe)] {
        assert!(
            preview
                .rgb
                .pixels()
                .zip(&pixels)
                .all(|(actual, input)| actual.0 == *input)
        );
        for (pixel, expected) in pixels.into_iter().zip(expected) {
            assert!((preview.luminance(pixel) - expected).abs() < 1e-12);
        }
    }
    let display = adobe.srgb().unwrap();
    let displayed_green = srgb.luminance(display.get_pixel(5, 0).0);
    assert!(
        displayed_green - adobe.luminance(pixels[5]) > 0.08,
        "display gamut clipping must not alter the metered Adobe RGB luminance"
    );
}

#[test]
fn adobe_rgb_display_conversion_does_not_reinterpret_or_mutate_metering_pixels() {
    let preview = native_rgb(&[[64, 128, 192]], 2);
    let before = preview.luminance([64, 128, 192]);
    let display = preview.srgb().unwrap();
    // Linear Adobe RGB -> XYZ D65 -> sRGB, then display clipping/encoding.
    for (actual, expected) in display.get_pixel(0, 0).0.into_iter().zip([0, 129, 196]) {
        assert!(actual.abs_diff(expected) <= 1);
    }
    assert_eq!(preview.rgb.get_pixel(0, 0).0, [64, 128, 192]);
    assert_eq!(preview.luminance([64, 128, 192]), before);
}

#[test]
fn adobe_preview_ifd_overrides_global_srgb_for_jpeg_and_rgb8() {
    let base =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
    for rgb in [false, true] {
        let data = if rgb {
            vec![89; 32 * 24 * 3]
        } else {
            jpeg([32, 24], 89)
        };
        // DNG PreviewColorSpace 3 is Adobe RGB, unlike LibRaw's enum value 2.
        let fixture = fixture::append(&base, [32, 24], &data, rgb, Some(3));
        let mut raw = crate::raw::RawFile::open(fixture.path()).unwrap();
        let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
        assert_eq!(thumbnail.color_space, 2);
        assert_eq!(thumbnail.rgb_dimensions, rgb.then_some([32, 24]));
        let preview = decode(thumbnail).unwrap().unwrap();
        assert!(preview.rgb.pixels().all(|p| p.0 == [89; 3]));
        assert!((preview.luminance([89; 3]) - 0.0987703894612559).abs() < 1e-12);
        assert!(
            raw.camera_image().is_err(),
            "preview access must not develop the RAW"
        );
    }
}

#[test]
fn missing_preview_orientation_inherits_capture_without_overriding_explicit_top_left() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
    let mut bytes = std::fs::read(source).unwrap();
    let root = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[root..root + 2].try_into().unwrap()) as usize;
    let entry = (0..count)
        .map(|i| root + 2 + 12 * i)
        .find(|&p| u16::from_le_bytes(bytes[p..p + 2].try_into().unwrap()) == 274)
        .unwrap();
    // Author a clockwise capture independently of the rendered IFD added below.
    bytes[entry + 8..entry + 10].copy_from_slice(&6u16.to_le_bytes());
    let base = crate::fixture_path::FixturePath::write("rotated-capture.dng", &bytes);
    for (orientation, flip, dimensions) in [(None, 6, (24, 32)), (Some(1), 0, (32, 24))] {
        let fixture = fixture::append_oriented(
            base.path(),
            [32, 24],
            &jpeg([32, 24], 89),
            false,
            Some(2),
            orientation,
        );
        let mut raw = crate::raw::RawFile::open(fixture.path()).unwrap();
        assert_eq!(
            raw.metadata.flip, 6,
            "the control capture must need rotation"
        );
        let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
        assert_eq!(thumbnail.flip, flip);
        assert_eq!(
            decode(thumbnail).unwrap().unwrap().rgb.dimensions(),
            dimensions
        );
    }
}

#[test]
fn embedded_icc_overrides_declared_space_and_still_controls_preview_luminance() {
    use image::ImageEncoder;
    let xy = |x, y| lcms2::CIExyY { x, y, Y: 1. };
    let primaries = lcms2::CIExyYTRIPLE {
        Red: xy(0.64, 0.33),
        Green: xy(0.30, 0.60),
        Blue: xy(0.15, 0.06),
    };
    let curve = lcms2::ToneCurve::new(1.);
    let profile =
        lcms2::Profile::new_rgb(&xy(0.3127, 0.3290), &primaries, &[&curve, &curve, &curve])
            .unwrap();
    let mut data = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut data, 100);
    encoder.set_icc_profile(profile.icc().unwrap()).unwrap();
    encoder
        .encode(&[128; 8 * 4 * 3], 8, 4, image::ExtendedColorType::Rgb8)
        .unwrap();
    for color_space in [2, 65535] {
        let preview = decode(crate::raw::EmbeddedThumbnail {
            data: data.clone(),
            rgb_dimensions: None,
            flip: 0,
            color_space,
        })
        .unwrap()
        .unwrap();
        let pixel = preview.rgb.get_pixel(0, 0).0;
        assert!(
            pixel.into_iter().all(|v| v.abs_diff(188) <= 1),
            "a linear ICC input must be encoded to sRGB, not treated as the declared Adobe RGB"
        );
        assert!((preview.luminance(pixel) - 128. / 255.).abs() < 0.006);
    }
}

fn jpeg(size: [u32; 2], value: u8) -> Vec<u8> {
    let mut data = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut data, 100)
        .encode(
            &vec![value; (size[0] * size[1] * 3) as usize],
            size[0],
            size[1],
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    data
}

#[test]
fn matching_preview_ifd_colour_overrides_unknown_global_exif_for_jpeg_and_rgb8() {
    let base = crate::raw::fixture::bayer_dng();
    let size = [32, 24];
    let encoded = jpeg(size, 89);
    let unlabelled = fixture::append(base.path(), size, &encoded, false, None);
    let mut raw = crate::raw::RawFile::open(unlabelled.path()).unwrap();
    let unknown = raw.embedded_thumbnail().unwrap().unwrap();
    assert_ne!(
        unknown.color_space, 1,
        "control must lack a global sRGB label"
    );
    assert!(decode(unknown).unwrap().is_none());
    for rgb in [false, true] {
        let data = if rgb {
            vec![89; (size[0] * size[1] * 3) as usize]
        } else {
            encoded.clone()
        };
        let path = fixture::append(base.path(), size, &data, rgb, Some(2));
        let mut raw = crate::raw::RawFile::open(path.path()).unwrap();
        let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
        assert_eq!(thumbnail.color_space, 1);
        assert_eq!(thumbnail.rgb_dimensions, rgb.then_some(size));
        let decoded = decode(thumbnail).unwrap().unwrap();
        assert_eq!(decoded.rgb.dimensions(), (32, 24));
        assert!(
            decoded
                .rgb
                .pixels()
                .all(|pixel| pixel.0.iter().all(|v| v.abs_diff(89) <= 1))
        );
        assert!(
            raw.camera_image().is_err(),
            "rendered preview extraction must not develop the sensor"
        );
    }
    let unlabeled_rgb = fixture::append(
        base.path(),
        size,
        &vec![89; (size[0] * size[1] * 3) as usize],
        true,
        None,
    );
    assert!(
        crate::raw::RawFile::open(unlabeled_rgb.path())
            .unwrap()
            .embedded_thumbnail()
            .unwrap()
            .is_none()
    );
    let global_srgb =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
    let global_control = fixture::append(&global_srgb, size, &encoded, false, None);
    assert_eq!(
        crate::raw::RawFile::open(global_control.path())
            .unwrap()
            .embedded_thumbnail()
            .unwrap()
            .unwrap()
            .color_space,
        1
    );
    let unsupported = fixture::append(&global_srgb, size, &encoded, false, Some(4));
    let mut raw = crate::raw::RawFile::open(unsupported.path()).unwrap();
    let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
    assert_ne!(thumbnail.color_space, 1);
    assert!(decode(thumbnail).unwrap().is_none());
}

#[test]
fn bounded_rgb8_decode_requires_exact_channels_dimensions_and_a_known_colour_space() {
    let data = |dimensions, bytes, colour| crate::raw::EmbeddedThumbnail {
        data: bytes,
        rgb_dimensions: Some(dimensions),
        flip: 0,
        color_space: colour,
    };
    assert!(decode(data([8, 8], vec![80; 8 * 8 * 3 - 1], 1)).is_err());
    assert!(decode(data([0, 8], vec![], 1)).is_err());
    assert!(decode(data([8193, 1], vec![], 1)).is_err());
    assert!(
        decode(data([8, 8], vec![80; 8 * 8 * 3], 65535))
            .unwrap()
            .is_none()
    );
}
