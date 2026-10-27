use super::*;

use crate::preview_fixture as fixture;

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
    let unsupported = fixture::append(&global_srgb, size, &encoded, false, Some(3));
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
