#![cfg(all(feature = "export", feature = "ocio-reference", target_os = "macos"))]
use antitone::{
    Error,
    color::{ap0_to_ap1, ap1_to_ap0},
    export::{self, ExrPrecision, ImageSize, Overwrite},
    processing::Response,
    profiles,
    rendering::{CpuRenderer, OutputTarget},
};
use exr::prelude::*;
use glam::DVec3;
use std::{fs, path::Path};
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::{CompressionMethod, Tag},
};

#[test]
fn tiff_reopens_with_exact_rendered_codes_lossless_compression_and_pinned_icc() {
    let directory = tempfile::tempdir().unwrap();
    let size = ImageSize {
        width: 7,
        height: 19,
    };
    let pixel = |x: u32, y: u32| DVec3::new(f64::from(x) * 0.2 - 0.1, f64::from(y) * 0.1, 0.18);
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.tif"));
        export::tiff(
            &path,
            size,
            target,
            Overwrite::Refuse,
            Response::Original,
            |x, y| Ok(pixel(x, y)),
        )
        .unwrap();
        let mut decoder = Decoder::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (size.width, size.height));
        assert_eq!(
            decoder.get_tag_u16_vec(Tag::BitsPerSample).unwrap(),
            vec![16; 3]
        );
        assert_eq!(
            decoder.get_tag_u32(Tag::Compression).unwrap(),
            u32::from(CompressionMethod::Deflate.to_u16())
        );
        assert_eq!(
            decoder.get_tag_u8_vec(Tag::Unknown(34675)).unwrap(),
            profiles::icc(target)
        );
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16")
        };
        let mut input = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                input.push(pixel(x, y));
            }
        }
        let expected: Vec<_> = target
            .encode_u16(&CpuRenderer::new(target).unwrap().render(&input).unwrap())
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn exr_reopens_in_ap0_with_signed_hdr_and_correct_channel_precision_and_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let inputs = [
        DVec3::ZERO,
        DVec3::ONE,
        DVec3::new(-0.1, 4.5, 0.25),
        DVec3::new(0.25, 1000.0, 0.1),
    ];
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let path = directory.path().join(format!("{precision:?}.exr"));
        export::exr(
            &path,
            ImageSize {
                width: 4,
                height: 1,
            },
            precision,
            Overwrite::Refuse,
            Response::Original,
            |x, _| Ok(inputs[x as usize]),
        )
        .unwrap();
        let image = read_first_flat_layer_from_file(&path).unwrap();
        let chroma = image.attributes.chromaticities.unwrap();
        assert_eq!(chroma.white, Vec2(0.32168, 0.33767));
        assert_eq!(chroma.green, Vec2(0.0, 1.0));
        assert_eq!(chroma.blue, Vec2(0.0001, -0.0770));
        assert_eq!(
            image
                .layer_data
                .attributes
                .other
                .get(&Text::from("antitoneProcessingModel")),
            Some(&AttributeValue::I32(4))
        );
        for channel in &image.layer_data.channel_data.list {
            let index = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!("unexpected channel"),
            };
            for (i, input) in inputs.iter().enumerate() {
                let expected = (ap1_to_ap0() * *input)[index];
                match (&channel.sample_data, precision) {
                    (FlatSamples::F16(data), ExrPrecision::Half) => {
                        assert_eq!(data[i], half::f16::from_f64(expected))
                    }
                    (FlatSamples::F32(data), ExrPrecision::Full) => {
                        assert_eq!(data[i], expected as f32)
                    }
                    _ => panic!("wrong precision"),
                }
            }
        }
    }
}

#[test]
fn export_errors_cancel_partial_output_and_never_replace_without_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("photo.tif");
    let size = ImageSize {
        width: 4,
        height: 17,
    };
    let sentinel = b"existing file";
    fs::write(&path, sentinel).unwrap();
    assert!(
        export::tiff(
            &path,
            size,
            OutputTarget::Srgb,
            Overwrite::Refuse,
            Response::Original,
            |_, _| Ok(DVec3::ONE)
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), sentinel);
    let cancel = |_: u32, y: u32| {
        if y >= 9 {
            Err(Error::Export("cancelled".into()))
        } else {
            Ok(DVec3::ONE)
        }
    };
    assert!(
        export::tiff(
            &path,
            size,
            OutputTarget::Srgb,
            Overwrite::Confirmed,
            Response::Original,
            cancel
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), sentinel);
    let new = directory.path().join("new.tif");
    assert!(
        export::tiff(
            &new,
            size,
            OutputTarget::Srgb,
            Overwrite::Refuse,
            Response::Original,
            cancel
        )
        .is_err()
    );
    assert!(!new.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    export::tiff(
        &path,
        size,
        OutputTarget::Srgb,
        Overwrite::Confirmed,
        Response::Original,
        |_, _| Ok(DVec3::ONE),
    )
    .unwrap();
    assert_ne!(fs::read(&path).unwrap(), sentinel);
}

#[test]
fn exr_refuses_nonfinite_half_overflow_and_full_overflow_without_final_files() {
    let directory = tempfile::tempdir().unwrap();
    let size = ImageSize {
        width: 1,
        height: 1,
    };
    for (precision, ap0) in [
        (ExrPrecision::Half, DVec3::splat(70000.0)),
        (ExrPrecision::Full, DVec3::splat(f64::from(f32::MAX) * 2.0)),
        (ExrPrecision::Half, DVec3::splat(f64::NAN)),
    ] {
        let path = directory.path().join("failed.exr");
        assert!(
            export::exr(
                &path,
                size,
                precision,
                Overwrite::Refuse,
                Response::Original,
                |_, _| Ok(ap0_to_ap1() * ap0)
            )
            .is_err()
        );
        assert!(!path.exists());
    }
    let path = directory.path().join("full.exr");
    export::exr(
        &path,
        size,
        ExrPrecision::Full,
        Overwrite::Refuse,
        Response::Original,
        |_, _| Ok(DVec3::splat(70000.0)),
    )
    .unwrap();
    assert!(Path::new(&path).exists());
}
