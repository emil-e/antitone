use super::*;

use crate::{
    geometry::SourceRaster,
    project::{Development, SourceDevelopment, SourceId},
};

fn captured(
    id: u64,
    area: crate::geometry::SourceRegion,
    size: [u32; 2],
    pixels: Vec<[f32; 3]>,
    scale: f64,
) -> CapturedSource {
    let input = crate::spatial_metadata::Input {
        source: SourceId(id),
        revision: crate::source::FileRevision::read(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        )
        .unwrap(),
        development: SourceDevelopment {
            decode: Development::new("authored-camera-raster".into(), [1.; 4]),
            dimensions: [area.x + area.width, area.y + area.height],
            active_area: [area.x, area.y, area.width, area.height],
            camera_model: "authored".into(),
            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
        },
        correction: None,
    };
    let camera = Arc::new(AnalysisInput::authored(
        SourceRaster::new(area, size).unwrap(),
        pixels,
        crate::processing::CAMERA_INPUT_FLOOR * scale,
    ));
    let key = crate::source::SourceKey::for_load(
        std::path::Path::new("authored"),
        crate::source::FileKey::unversioned(std::path::Path::new("authored")),
        &input.development,
    );
    let camera = crate::source::CameraSamples::authored(key, input.development.clone(), camera);
    CapturedSource { input, camera }
}

#[test]
fn geometry_preserves_camera_units_fractions_and_hdr_independently_of_peer_precision() {
    let area = [11, 17, 1, 1].into();
    let first = captured(1, area, [1, 1], vec![[0.25, 8000.125, 65535.]], 1.);
    let alone = geometry_frames(
        vec![CapturedSource {
            input: first.input.clone(),
            camera: first.camera.clone(),
        }],
        &|| Ok(()),
    )
    .unwrap();
    let frames = geometry_frames(
        vec![
            first,
            captured(2, area, [1, 1], vec![[262140.5, 131068.25, 0.]], 4.),
        ],
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].input.source, SourceId(1));
    assert_eq!(frames[1].input.source, SourceId(2));
    assert_eq!(frames[0].size, [1, 1]);
    assert_eq!(frames[0].rgb, [[0.25, 8000.125, 65535.]]);
    assert_eq!(frames[1].rgb, [[262140.5, 131068.25, 0.]]);
    assert_eq!(frames[0].rgb, alone[0].rgb);
}

#[test]
fn geometry_resampling_preserves_full_source_extent_and_affine_colour() {
    let area = [101, 53, 1536, 1024].into();
    let size = [1024, 683];
    let mut pixels = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let u = (f64::from(x) + 0.5) / f64::from(size[0]);
            let v = (f64::from(y) + 0.5) / f64::from(size[1]);
            pixels.push([(262140. * u) as f32, (131068. * v) as f32, 65535.]);
        }
    }
    let camera = captured(1, area, size, pixels, 4.);
    let frames = geometry_frames(vec![camera], &|| Ok(())).unwrap();
    let frame = &frames[0];
    assert_eq!(frame.input.development.active_area, [101, 53, 1536, 1024]);
    assert_eq!(frame.size, [768, 512]);
    assert_eq!(frame.rgb.len(), 768 * 512);
    for [x, y] in [[0_u32, 0], [767, 0], [0, 511], [767, 511], [333, 171]] {
        // An affine field has this value at each output cell centre. The tolerance covers
        // rounding the authored samples to f32 and narrowing the interpolated result once.
        let expected = [
            262140. * (f64::from(x) + 0.5) / 768.,
            131068. * (f64::from(y) + 0.5) / 512.,
            65535.,
        ];
        for (actual, expected) in frame.rgb[(y * 768 + x) as usize].iter().zip(expected) {
            assert!(
                (f64::from(*actual) - expected).abs()
                    <= 2. * f64::from(f32::EPSILON) * expected.abs(),
                "({x}, {y}): {actual} != {expected}"
            );
        }
    }
}

#[test]
fn geometry_keeps_dark_sites_that_are_ineligible_for_numerical_analysis() {
    let frame = captured(
        1,
        [13, 29, 3, 1].into(),
        [3, 1],
        vec![[0.; 3], [8.; 3], [12.; 3]],
        16.,
    );
    for i in 0..3 {
        assert!(!frame.camera.pixels().above_native_floor(i).unwrap());
    }
    let frames = geometry_frames(vec![frame], &|| Ok(())).unwrap();
    assert_eq!(frames[0].size, [3, 1]);
    assert_eq!(frames[0].rgb, [[0.; 3], [8.; 3], [12.; 3]]);
}

#[test]
fn retired_geometry_preparation_stops_before_sampling() {
    let frame = captured(1, [0, 0, 1, 1].into(), [1, 1], vec![[1000.; 3]], 1.);
    assert!(geometry_frames(vec![frame], &|| Err("retired".into())).is_err());
}
