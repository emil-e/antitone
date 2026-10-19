#![cfg(all(feature = "raw", target_os = "macos"))]
use antitone::{
    geometry::SourceRaster,
    raw::{DevelopmentPriority, RawFile, Rect, SensorDiagnostics},
    scan::Scan,
};
use glam::{DMat3, DVec3};
use std::{fs, path::PathBuf};

const WB: [f64; 4] = [1.; 4];
const PATCHES: [[u16; 3]; 12] = [
    [3000, 3000, 3000],
    [128, 128, 128],
    [1000, 1000, 1000],
    [2400, 1200, 600],
    [1200, 2400, 600],
    [600, 1200, 2400],
    [3200, 3200, 3200],
    [600, 1000, 1800],
    [256, 256, 256],
    [512, 512, 512],
    [1024, 1024, 1024],
    [2048, 2048, 2048],
];

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")
}

fn expected_diagnostics() -> SensorDiagnostics {
    let mut expected = SensorDiagnostics::default();
    for y in 0..192 {
        for x in 0..256 {
            // LibRaw's immutable CFA distinguishes the two green sites in slots 1 and 3.
            let c = [[0, 1], [3, 2]][y % 2][x % 2];
            let channel = if c == 3 { 1 } else { c };
            let code = u32::from(PATCHES[(y / 64) * 4 + x / 64][channel]) + 64;
            expected.sites[c] += 1;
            expected.peak[c] = expected.peak[c].max(code);
        }
    }
    expected
}

#[test]
fn authored_dng_libraw_metadata_sensor_values_and_complete_rgb16_survive_development() {
    let path = fixture();
    let original = fs::read(&path).unwrap();
    let mut raw = RawFile::open_uncorrected(&path).unwrap();
    let metadata = raw.metadata;
    assert_eq!(metadata.make(), "Antitone");
    assert_eq!(metadata.model(), "Authored Bayer patches");
    assert_eq!((metadata.raw_width, metadata.raw_height), (256, 192));
    assert_eq!(
        (
            metadata.width,
            metadata.height,
            metadata.raw_bps,
            metadata.colors
        ),
        (256, 192, 16, 3)
    );
    assert_eq!((metadata.black, metadata.maximum), (64, 4095));
    assert_eq!(metadata.cblack, [0; 4]);
    assert_eq!(&metadata.cam_mul[..3], &[1.; 3]);
    assert!(
        (metadata.camera_to_srgb() - DMat3::IDENTITY)
            .to_cols_array()
            .iter()
            .all(|v| v.abs() < 2e-6)
    );
    assert!(metadata.camera_to_ap1().unwrap().determinant().abs() > 0.1);
    assert_eq!(raw.unpack().unwrap(), expected_diagnostics());
    let region = Rect {
        x: 16,
        y: 16,
        width: 32,
        height: 32,
    };
    let before = raw.region_sensor_diagnostics(region).unwrap();
    assert_eq!(before.sites.iter().sum::<u64>(), 1024);
    assert_eq!(before.sites, [256; 4]);
    assert_eq!(before.peak, [3064; 4]);
    // Explicit roll-fixed WB is a supported API; DNG's unused fourth metadata gain is zero.
    let developed = raw.develop_camera(WB).unwrap();
    let mut codes = Vec::new();
    for y in 0..192 {
        for x in 0..256 {
            codes.push(developed.pixel_codes(x, y).unwrap());
        }
    }
    assert_eq!(std::mem::size_of_val(codes.as_slice()), 256 * 192 * 8);
    for (index, patch) in PATCHES.iter().enumerate() {
        let p = developed
            .pixel((index % 4) as u32 * 64 + 32, (index / 4) as u32 * 64 + 32)
            .unwrap();
        let expected = DVec3::new(patch[0] as f64, patch[1] as f64, patch[2] as f64) / 4031.;
        assert!(
            (p - expected).abs().max_element() <= 2. / 65535.,
            "{index}: {p:?} != {expected:?}"
        );
    }
    // Reopen with the background policy rather than reusing the cached camera image.
    let mut background = RawFile::open_uncorrected(&path).unwrap();
    background.set_development_priority(DevelopmentPriority::Background);
    let repeated = background.develop_camera(WB).unwrap();
    for y in 0..192 {
        for x in 0..256 {
            assert_eq!(
                repeated.pixel_codes(x, y).unwrap(),
                codes[(y * 256 + x) as usize]
            );
        }
    }
    assert_eq!(raw.region_sensor_diagnostics(region).unwrap(), before);
    assert_eq!(
        raw.region_sensor_diagnostics(Rect {
            x: 0,
            y: 0,
            width: 256,
            height: 192
        })
        .unwrap(),
        expected_diagnostics()
    );
    assert_eq!(raw.unpack().unwrap(), expected_diagnostics());
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn authored_dng_automatic_wb_matches_explicit_development_without_changing_metadata() {
    let path = fixture();
    let mut raw = RawFile::open(&path).unwrap();
    assert_eq!(raw.metadata.cam_mul, [1., 1., 1., 0.]);
    assert_eq!(raw.metadata.as_shot_wb(), WB);
    assert_eq!(raw.metadata.cam_mul, [1., 1., 1., 0.]);
    let automatic = Scan::open(&path, None).unwrap();
    let explicit = Scan::open(&path, Some(WB)).unwrap();
    assert_eq!(automatic.fixed_wb, WB);
    assert_eq!(
        automatic.preview_source(1024).unwrap(),
        explicit.preview_source(1024).unwrap()
    );
    let custom = if raw.metadata.dng_identity().is_some() {
        [2., 1., 1.5, 1.]
    } else {
        [2., 1., 1.5, 0.75]
    };
    let custom_scan = Scan::open(&path, Some(custom)).unwrap();
    assert_eq!(custom_scan.fixed_wb, custom);
    let mut direct = RawFile::open(&path).unwrap();
    let direct = direct.develop_camera(custom).unwrap();
    for [x, y] in [[32, 32], [224, 32], [32, 96], [96, 96]] {
        assert_eq!(
            custom_scan.pixel(x, y).unwrap(),
            direct.pixel(x, y).unwrap()
        );
        assert_ne!(
            custom_scan.pixel(x, y).unwrap(),
            automatic.pixel(x, y).unwrap()
        );
    }
    let before = raw
        .develop_camera(WB)
        .unwrap()
        .pixel_samples(32, 32)
        .unwrap();
    for channel in 0..4 {
        for value in [0., -1., f64::NAN, f64::INFINITY, f64::MAX] {
            let mut invalid = WB;
            invalid[channel] = value;
            assert!(raw.develop_camera(invalid).is_err());
            assert_eq!(
                raw.camera_image().unwrap().pixel_samples(32, 32).unwrap(),
                before
            );
        }
    }
    assert!(Scan::open(&path, Some([1., 1., 1., 0.])).is_err());
}

#[test]
fn authored_dng_scan_keeps_crop_proxy_centres_and_original_reference_values() {
    let mut scan = Scan::open(&fixture(), Some(WB)).unwrap();
    assert_eq!(scan.dimensions().unwrap(), (256, 192));
    assert_eq!(
        scan.area,
        Rect {
            x: 8,
            y: 8,
            width: 240,
            height: 176
        }
    );
    let (w, h, codes) = scan.preview_source(60).unwrap();
    assert_eq!((w, h), (60, 44));
    assert_eq!(scan.preview_source(60).unwrap(), (w, h, codes.clone()));
    let raster = SourceRaster::new(scan.area, [w, h]).unwrap();
    for y in 0..h {
        for x in 0..w {
            let [sx, sy] = raster.source_pixel([x, y]).unwrap();
            assert_eq!([sx, sy], [8 + 4 * x + 2, 8 + 4 * y + 2]);
            let expected = DVec3::new(
                codes[(y * w + x) as usize][0] as f64,
                codes[(y * w + x) as usize][1] as f64,
                codes[(y * w + x) as usize][2] as f64,
            ) / 65535.;
            assert_eq!(scan.pixel(sx, sy).unwrap(), expected);
        }
    }
    let patch = Rect {
        x: 16,
        y: 16,
        width: 32,
        height: 32,
    };
    let mean = scan.reference(patch).unwrap();
    assert!((mean - DVec3::splat(3000. / 4031.)).abs().max_element() <= 2. / 65535.);
    assert_eq!(scan.reference(patch).unwrap(), mean);
    assert!(
        scan.reference(Rect {
            x: 256,
            y: 0,
            width: 32,
            height: 32
        })
        .is_err()
    );
    assert_eq!(scan.reference(patch).unwrap(), mean);
    assert_eq!(scan.preview_source(60).unwrap(), (w, h, codes));
}

#[test]
fn authored_raw_keeps_the_measured_anchor_flooring_and_libraw_matrix_gates() {
    use antitone::processing::{Calibration, Inversion, RollSettings};
    let mut scan = Scan::open(&fixture(), None).unwrap();
    let calibration = Calibration {
        dmin: scan
            .reference(Rect {
                x: 16,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        dmax: scan
            .reference(Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        input_floor: scan.input_floor,
    }
    .validate()
    .unwrap();
    for gamma in [0.1, 0.65, 1., 3.] {
        let inversion = Inversion::new(
            calibration,
            RollSettings {
                gamma,
                ..RollSettings::default()
            },
        )
        .unwrap();
        assert!(
            inversion
                .apply(calibration.dmin)
                .unwrap()
                .rgb
                .abs()
                .max_element()
                <= 2e-12
        );
        assert!(
            (inversion.apply(calibration.dmax).unwrap().rgb - DVec3::ONE)
                .abs()
                .max_element()
                <= 2e-12
        );
    }
    let mut raw = RawFile::open_uncorrected(&fixture()).unwrap();
    let matrix = raw.metadata.camera_to_srgb();
    let area = raw.metadata.image_area().unwrap();
    let image = raw.develop_camera(WB).unwrap();
    let mut below_floor = [0u64; 3];
    let mut points = Vec::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let camera = image.pixel(x, y).unwrap();
            for c in 0..3 {
                below_floor[c] += u64::from(camera[c] < scan.input_floor);
            }
            if (x - area.x).is_multiple_of(32) && (y - area.y).is_multiple_of(32) {
                points.push((x, y, matrix * camera));
            }
        }
    }
    let count = u64::from(area.width) * u64::from(area.height);
    assert!(below_floor.into_iter().all(|v| v <= count / 100));
    let reference = raw.develop_srgb_reference(WB).unwrap();
    let mut compared = 0;
    for (x, y, expected) in points {
        if expected.min_element() < 0. || expected.max_element() > 1. {
            continue;
        }
        assert!(
            (reference.pixel(x, y).unwrap() - expected)
                .abs()
                .max_element()
                <= 2. / 65535.
        );
        compared += 1;
    }
    // The diagnostic permits an empty eligible grid; this fixture must actually exercise it.
    assert!(compared > 0);
}

#[cfg(any(feature = "export", feature = "project"))]
#[test]
fn authored_raw_rejects_sensor_white_and_black_references_without_mutating_source() {
    let original = fs::read(fixture()).unwrap();
    let ifd = u32::from_le_bytes(original[4..8].try_into().unwrap()) as usize;
    let entries = u16::from_le_bytes(original[ifd..ifd + 2].try_into().unwrap()) as usize;
    let strip = (0..entries)
        .map(|i| ifd + 2 + i * 12)
        .find(|i| u16::from_le_bytes(original[*i..*i + 2].try_into().unwrap()) == 273)
        .unwrap();
    let offset = u32::from_le_bytes(original[strip + 8..strip + 12].try_into().unwrap()) as usize;
    let directory = tempfile::tempdir().unwrap();
    let patch = Rect {
        x: 16,
        y: 16,
        width: 32,
        height: 32,
    };
    for code in [4095u16, 64u16] {
        let mut bytes = original.clone();
        for y in 16..48 {
            for x in 16..48 {
                let at = offset + (y * 256 + x) * 2;
                bytes[at..at + 2].copy_from_slice(&code.to_le_bytes());
            }
        }
        let path = directory.path().join(format!("{code}.dng"));
        fs::write(&path, &bytes).unwrap();
        let mut scan = Scan::open(&path, None).unwrap();
        let before = scan.preview_source(60).unwrap();
        let capture = scan.source().capture_eligibility(patch).unwrap();
        assert_eq!(
            (capture.at_white, capture.below_black),
            (code == 4095, code == 64)
        );
        assert!(matches!(
            scan.reference(patch),
            Err(antitone::Error::UnusableSample(_))
        ));
        assert_eq!(scan.preview_source(60).unwrap(), before);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert_eq!(fs::read(fixture()).unwrap(), original);
}

#[cfg(all(feature = "export", feature = "ocio-reference"))]
fn authored_export_image() -> (
    antitone::export::ImageSize,
    antitone::processing::Response,
    Vec<DVec3>,
) {
    use antitone::{
        export::ImageSize,
        processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    };
    let mut scan = Scan::open(&fixture(), Some(WB)).unwrap();
    let calibration = Calibration {
        dmin: scan
            .reference(Rect {
                x: 16,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        dmax: scan
            .reference(Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        input_floor: scan.input_floor,
    };
    let roll = RollSettings::new_grade();
    let pipeline = Pipeline::new(
        calibration,
        roll,
        PhotoSettings::default(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let mut working = Vec::new();
    for y in scan.area.y..scan.area.y + scan.area.height {
        for x in scan.area.x..scan.area.x + scan.area.width {
            working.push(pipeline.apply(scan.pixel(x, y).unwrap()).unwrap().rgb);
        }
    }
    assert!(working.iter().any(|v| v.max_element() > 1.));
    assert!(working.iter().any(|v| v.min_element() < 0.));
    let size = ImageSize {
        width: scan.area.width,
        height: scan.area.height,
    };
    (size, roll.response, working)
}

#[cfg(all(feature = "export", feature = "ocio-reference"))]
#[test]
fn authored_dng_model4_exports_reopen_as_rgb16_tiff_and_ap0_exr() {
    use antitone::{
        color::ap1_to_ap0,
        export::{self, ExrPrecision, Overwrite},
        profiles,
        rendering::{CpuRenderer, OutputTarget},
    };
    use exr::prelude::{FlatSamples, read_first_flat_layer_from_file};
    use tiff::{
        decoder::{Decoder, DecodingResult},
        tags::Tag,
    };
    let (size, response, working) = authored_export_image();
    let pixel = |x: u32, y: u32| Ok(working[(y * size.width + x) as usize]);
    let directory = tempfile::tempdir().unwrap();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.tif"));
        export::tiff(&path, size, target, Overwrite::Refuse, response, pixel).unwrap();
        let mut decoder = Decoder::new(fs::File::open(path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (size.width, size.height));
        assert_eq!(
            decoder.get_tag_u16_vec(Tag::BitsPerSample).unwrap(),
            vec![16; 3]
        );
        assert_eq!(
            decoder.get_tag_u8_vec(Tag::Unknown(34675)).unwrap(),
            profiles::icc(target)
        );
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16")
        };
        let expected: Vec<_> = target
            .encode_u16(&CpuRenderer::new(target).unwrap().render(&working).unwrap())
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(actual, expected);
    }
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let path = directory.path().join(format!("{precision:?}.exr"));
        export::exr(&path, size, precision, Overwrite::Refuse, response, pixel).unwrap();
        let image = read_first_flat_layer_from_file(path).unwrap();
        assert_eq!(
            image.layer_data.size,
            exr::math::Vec2(size.width as usize, size.height as usize)
        );
        assert_eq!(
            image.attributes.chromaticities.unwrap().white,
            exr::math::Vec2(0.32168, 0.33767)
        );
        for channel in &image.layer_data.channel_data.list {
            let index = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!("unexpected channel"),
            };
            for (i, p) in working.iter().enumerate() {
                let expected = (ap1_to_ap0() * *p)[index];
                match (&channel.sample_data, precision) {
                    (FlatSamples::F16(data), ExrPrecision::Half) => {
                        assert_eq!(data[i], half::f16::from_f64(expected))
                    }
                    (FlatSamples::F32(data), ExrPrecision::Full) => {
                        assert_eq!(data[i], expected as f32)
                    }
                    _ => panic!("wrong channel precision"),
                }
            }
        }
    }
}

#[cfg(all(feature = "export", feature = "ocio-reference"))]
#[test]
#[ignore = "independent macOS ImageIO reference; compiles with Apple Clang"]
fn authored_dng_tiffs_reopen_with_independent_imageio_storage_and_profiles() {
    use antitone::{
        export::{self, ImageSize, Overwrite},
        rendering::OutputTarget,
    };
    use std::{path::Path, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let checker = directory.path().join("tiff-reference");
    let compiled = Command::new("/usr/bin/clang++")
        .args(["-std=c++17", "-O2"])
        .arg(root.join("native/tiff_reference.mm"))
        .args([
            "-framework",
            "Foundation",
            "-framework",
            "CoreGraphics",
            "-framework",
            "ImageIO",
            "-o",
        ])
        .arg(&checker)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let check = |path: &Path, width: u32, height: u32, profile: &Path| {
        Command::new(&checker)
            .arg(path)
            .arg(width.to_string())
            .arg(height.to_string())
            .arg(profile)
            .output()
            .unwrap()
    };
    let (size, response, working) = authored_export_image();
    let small = ImageSize {
        width: 7,
        height: 19,
    };
    let mut synthetic = Vec::new();
    for y in 0..small.height {
        for x in 0..small.width {
            synthetic.push(DVec3::new(
                f64::from(x) * 0.2 - 0.1,
                f64::from(y) * 0.1,
                0.18,
            ));
        }
    }
    for (size, working) in [(size, working), (small, synthetic)] {
        for (suffix, other, target) in [
            ("srgb", "display-p3", OutputTarget::Srgb),
            ("display-p3", "srgb", OutputTarget::DisplayP3),
        ] {
            let path = directory
                .path()
                .join(format!("{}x{}-{suffix}.tif", size.width, size.height));
            let profile = root.join(format!("assets/profiles/antitone-{suffix}-v1.icc"));
            export::tiff(&path, size, target, Overwrite::Refuse, response, |x, y| {
                Ok(working[(y * size.width + x) as usize])
            })
            .unwrap();
            let result = check(&path, size.width, size.height, &profile);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!("{}", String::from_utf8_lossy(&result.stdout).trim());
            assert!(String::from_utf8_lossy(&result.stdout).contains("pinned_ICC=exact"));
            let wrong_size = check(&path, size.width + 1, size.height, &profile);
            assert_eq!(wrong_size.status.code(), Some(3), "{wrong_size:?}");
            let wrong_profile = check(
                &path,
                size.width,
                size.height,
                &root.join(format!("assets/profiles/antitone-{other}-v1.icc")),
            );
            assert_eq!(wrong_profile.status.code(), Some(4), "{wrong_profile:?}");
        }
    }
    let profile = root.join("assets/profiles/antitone-srgb-v1.icc");
    let malformed = directory.path().join("malformed.tif");
    fs::write(&malformed, b"not a TIFF").unwrap();
    assert_eq!(check(&malformed, 1, 1, &profile).status.code(), Some(2));
    let rgb8 = directory.path().join("rgb8.tif");
    tiff::encoder::TiffEncoder::new(fs::File::create(&rgb8).unwrap())
        .unwrap()
        .write_image::<tiff::encoder::colortype::RGB8>(1, 1, &[64, 128, 192])
        .unwrap();
    assert_eq!(check(&rgb8, 1, 1, &profile).status.code(), Some(3));
    let multiple = directory.path().join("multiple.tif");
    let mut encoder =
        tiff::encoder::TiffEncoder::new(fs::File::create(&multiple).unwrap()).unwrap();
    for _ in 0..2 {
        encoder
            .write_image::<tiff::encoder::colortype::RGB16>(1, 1, &[0; 3])
            .unwrap();
    }
    drop(encoder);
    assert_eq!(check(&multiple, 1, 1, &profile).status.code(), Some(2));
    assert_eq!(check(&rgb8, 0, 1, &profile).status.code(), Some(1));
    assert_eq!(
        check(&rgb8, 1, 1, &directory.path().join("missing.icc"))
            .status
            .code(),
        Some(1)
    );
}
