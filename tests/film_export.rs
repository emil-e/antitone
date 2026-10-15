#![cfg(all(feature = "export", feature = "ocio-reference", target_os = "macos"))]
use antitone::{
    color::ap1_to_ap0,
    export::{self, ExrPrecision, ImageSize, Overwrite},
    processing::{Calibration, PhotoSettings, Pipeline, Response, RollSettings},
    rendering::{CpuRenderer, OutputTarget},
};
use exr::prelude::*;
use glam::DVec3;
use std::fs::File;
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::Tag,
};
#[test]
fn film_exports_reopen_with_actual_response_and_unchanged_target_and_ap0_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let response = Response::Film {
        toe: 2.543210987654321,
        shoulder: 0.003210987654321,
    };
    let roll = RollSettings {
        response,
        exposure: 2.345678901,
        ..RollSettings::default()
    };
    let cal = Calibration {
        dmin: DVec3::new(0.1685663501546697, 0.13359162596075455, 0.11715123642832656),
        dmax: DVec3::new(0.008061, 0.003409, 0.001442),
        input_floor: 2.1630933622265155e-5,
    };
    let photo = PhotoSettings {
        contrast: 1.2,
        saturation: 1.3,
        ..PhotoSettings::default()
    };
    let pipeline = Pipeline::new(cal, roll, photo, glam::DMat3::IDENTITY).unwrap();
    let scans = [
        cal.dmin * (1. - 1e-12),
        cal.dmin * (1. + 1e-12),
        cal.dmin * 0.5,
        cal.dmax,
        DVec3::ZERO,
    ];
    let working: Vec<_> = scans
        .into_iter()
        .map(|s| pipeline.apply(s).unwrap().rgb)
        .collect();
    let size = ImageSize {
        width: 5,
        height: 1,
    };
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.tif"));
        export::tiff(&path, size, target, Overwrite::Refuse, response, |x, _| {
            Ok(working[x as usize])
        })
        .unwrap();
        let mut decoder = Decoder::new(File::open(&path).unwrap()).unwrap();
        let description = decoder.get_tag_ascii_string(Tag::ImageDescription).unwrap();
        assert!(description.contains("Antitone model 4"));
        assert!(description.contains("native-camera RGB inversion to AP1"));
        assert!(description.contains("toe=2.543210987654321"));
        assert!(description.contains("shoulder=0.003210987654321"));
        assert!(description.contains("signed-root"));
        assert!(description.contains(antitone::photographic::CONTRACT));
        assert!(!description.contains(antitone::local_adjustments::CONTRACT));
        let expected: Vec<_> = target
            .encode_u16(&CpuRenderer::new(target).unwrap().render(&working).unwrap())
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16")
        };
        assert_eq!(actual, expected);
    }
    let path = directory.path().join("film.exr");
    export::exr(
        &path,
        size,
        ExrPrecision::Full,
        Overwrite::Refuse,
        response,
        |x, _| Ok(working[x as usize]),
    )
    .unwrap();
    let image = read_first_flat_layer_from_file(&path).unwrap();
    let attrs = &image.layer_data.attributes.other;
    assert_eq!(
        attrs.get(&Text::from("antitoneProcessingModel")),
        Some(&AttributeValue::I32(4))
    );
    assert_eq!(
        attrs.get(&Text::from("antitoneInversionSpace")),
        Some(&AttributeValue::Text(Text::from(
            "native-camera RGB; post-inversion metadata camera-to-AP1"
        )))
    );
    assert_eq!(
        attrs.get(&Text::from("antitoneFilmResponse")),
        Some(&AttributeValue::Text(Text::from(
            "toe=2.543210987654321; shoulder=0.003210987654321; signed-root continuation above base"
        )))
    );
    assert_eq!(
        attrs.get(&Text::from("antitonePhotographicProcessing")),
        Some(&AttributeValue::Text(Text::from(
            antitone::photographic::CONTRACT
        )))
    );
    assert!(!attrs.contains_key(&Text::from("antitoneLocalAdjustmentProcessing")));
    for channel in &image.layer_data.channel_data.list {
        let c = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected channel"),
        };
        let FlatSamples::F32(values) = &channel.sample_data else {
            panic!("not full float")
        };
        for (i, w) in working.iter().enumerate() {
            assert_eq!(values[i], (ap1_to_ap0() * *w)[c] as f32);
        }
    }
}
#[test]
fn invalid_response_metadata_cannot_create_an_export() {
    let directory = tempfile::tempdir().unwrap();
    let response = Response::Film {
        toe: 2.5,
        shoulder: 0.021,
    };
    let size = ImageSize {
        width: 1,
        height: 1,
    };
    let path = directory.path().join("invalid.tif");
    assert!(
        export::tiff(
            &path,
            size,
            OutputTarget::DisplayP3,
            Overwrite::Refuse,
            response,
            |_, _| Ok(DVec3::ONE)
        )
        .is_err()
    );
    assert!(!path.exists());
    let path = directory.path().join("invalid.exr");
    assert!(
        export::exr(
            &path,
            size,
            ExrPrecision::Full,
            Overwrite::Refuse,
            response,
            |_, _| Ok(DVec3::ONE)
        )
        .is_err()
    );
    assert!(!path.exists());
}
