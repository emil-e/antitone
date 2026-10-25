#![cfg(all(feature = "raw", feature = "export", feature = "ocio-reference"))]
use antitone::{
    color::ap1_to_ap0,
    export::{self, ExrPrecision, ImageSize, Overwrite},
    processing::{Calibration, InputSettings, PhotoSettings, Pipeline, RollSettings},
    profiles,
    raw::Rect,
    rendering::{CpuRenderer, OutputTarget},
    scan::Scan,
};
use glam::DVec3;
use std::{path::Path, sync::Arc, time::Instant};

#[test]
#[ignore = "full-size read-only local RAW export acceptance; run explicitly in release"]
fn corrected_full_size_tiff_and_ap0_exr_reopen() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut reference = Scan::open(&root.join("samples/DSC01739.ARW"), None).unwrap();
    let support = reference.propose_flat_field_area().unwrap().unwrap();
    let map = Arc::new(reference.estimate_flat_field(support).unwrap());
    let mut scan =
        Scan::open(&root.join("samples/DSC01737.ARW"), Some(reference.fixed_wb)).unwrap();
    scan.set_flat_field(Some(map)).unwrap();
    let a = scan.area;
    // Diagnostic anchors exercise signed/HDR export, not photographic film calibration.
    let dmin = scan
        .reference(Rect {
            x: a.x + a.width / 2 - 128,
            y: a.y + a.height / 2 - 128,
            width: 256,
            height: 256,
        })
        .unwrap();
    let calibration = scan
        .processing_calibration(Calibration {
            dmin,
            dmax: dmin * 0.1,
            input_floor: scan.input_floor,
        })
        .unwrap();
    let roll = RollSettings {
        exposure: 2.,
        ..RollSettings::default()
    };
    let input = InputSettings::negative(calibration);
    let pipeline =
        Pipeline::new(input, roll, PhotoSettings::default(), scan.camera_to_ap1).unwrap();
    let size = ImageSize {
        width: a.width,
        height: a.height,
    };
    let pixel = |x, y| pipeline.apply(scan.pixel(a.x + x, a.y + y)?).map(|p| p.rgb);
    let positions: Vec<_> = (0..13)
        .flat_map(|y| (0..19).map(move |x| [x * (a.width - 1) / 18, y * (a.height - 1) / 12]))
        .collect();
    let working: Vec<DVec3> = positions
        .iter()
        .map(|[x, y]| pixel(*x, *y).unwrap())
        .collect();
    let directory = tempfile::tempdir().unwrap();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let start = Instant::now();
        let path = directory.path().join(format!("{target:?}.tif"));
        export::tiff(&path, size, target, Overwrite::Refuse, pixel).unwrap();
        let mut limits = tiff::decoder::Limits::default();
        limits.decoding_buffer_size = a.width as usize * a.height as usize * 6;
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&path).unwrap())
            .unwrap()
            .with_limits(limits);
        assert_eq!(decoder.dimensions().unwrap(), (a.width, a.height));
        assert_eq!(
            decoder.get_tag_u32(tiff::tags::Tag::Compression).unwrap(),
            5
        );
        assert_eq!(decoder.get_tag_u32(tiff::tags::Tag::Predictor).unwrap(), 2);
        assert_eq!(
            decoder
                .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                .unwrap(),
            profiles::icc(target)
        );
        let tiff::decoder::DecodingResult::U16(data) = decoder.read_image().unwrap() else {
            panic!("not RGB16");
        };
        assert_eq!(data.len(), a.width as usize * a.height as usize * 3);
        let expected = target
            .encode_u16(&CpuRenderer::new(target).unwrap().render(&working).unwrap())
            .unwrap();
        for ([x, y], rgb) in positions.iter().zip(expected) {
            let i = (*y as usize * a.width as usize + *x as usize) * 3;
            assert_eq!(&data[i..i + 3], &rgb);
        }
        println!(
            "full_flat_tiff target={target:?} size={}x{} reopened_pixels={} exact_checks={} bytes={} seconds={:.3}",
            a.width,
            a.height,
            data.len() / 3,
            positions.len(),
            std::fs::metadata(path).unwrap().len(),
            start.elapsed().as_secs_f64()
        );
    }
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let start = Instant::now();
        let path = directory.path().join(format!("{precision:?}.exr"));
        export::exr(&path, size, precision, Overwrite::Refuse, pixel).unwrap();
        let image = exr::prelude::read_first_flat_layer_from_file(&path).unwrap();
        assert_eq!(
            image.layer_data.size,
            exr::math::Vec2(a.width as usize, a.height as usize)
        );
        assert_eq!(
            image.attributes.chromaticities.unwrap().white,
            exr::math::Vec2(0.32168, 0.33767)
        );
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for channel in &image.layer_data.channel_data.list {
            let c = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!("unexpected channel"),
            };
            for sample in channel.sample_data.values_as_f32() {
                assert!(sample.is_finite());
                min = min.min(sample as f64);
                max = max.max(sample as f64);
            }
            for ([x, y], rgb) in positions.iter().zip(&working) {
                let i = *y as usize * a.width as usize + *x as usize;
                let expected = (ap1_to_ap0() * *rgb)[c];
                match (&channel.sample_data, precision) {
                    (exr::prelude::FlatSamples::F16(data), ExrPrecision::Half) => {
                        assert_eq!(data[i], half::f16::from_f64(expected))
                    }
                    (exr::prelude::FlatSamples::F32(data), ExrPrecision::Full) => {
                        assert_eq!(data[i], expected as f32)
                    }
                    _ => panic!("wrong precision"),
                }
            }
        }
        assert!(
            min < 0. && max > 1.,
            "diagnostic image must exercise signed/HDR AP0: {min}..{max}"
        );
        println!(
            "full_flat_exr precision={precision:?} size={}x{} exact_checks={} range={min}..{max} bytes={} seconds={:.3}",
            a.width,
            a.height,
            positions.len(),
            std::fs::metadata(path).unwrap().len(),
            start.elapsed().as_secs_f64()
        );
    }
}
