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
    CapturedSource {
        input: crate::spatial_metadata::Input {
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
        },
        camera: Arc::new(AnalysisInput::authored(
            SourceRaster::new(area, size).unwrap(),
            pixels,
            crate::processing::CAMERA_INPUT_FLOOR * scale,
        )),
    }
}

#[test]
fn shared_rasters_use_one_encoding_scale_without_clipping_hdr() {
    let area = [11, 17, 1, 1].into();
    let frames = geometry_frames(
        vec![
            captured(1, area, [1, 1], vec![[4000., 8000., 65535.]], 1.),
            captured(2, area, [1, 1], vec![[262140., 131068., 0.]], 4.),
        ],
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].input.source, SourceId(1));
    assert_eq!(frames[1].input.source, SourceId(2));
    assert_eq!(frames[0].size, [1, 1]);
    assert_eq!(frames[0].rgb, [[1000, 2000, 16384]]);
    assert_eq!(frames[1].rgb, [[65535, 32767, 0]]);
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
        // An affine camera field has this value at each output cell centre. One encoded
        // code covers f32 interpolation error around the final integer rounding boundary.
        let expected = [
            (65535. * (f64::from(x) + 0.5) / 768.).round() as u16,
            (32767. * (f64::from(y) + 0.5) / 512.).round() as u16,
            16384,
        ];
        for (actual, expected) in frame.rgb[(y * 768 + x) as usize].iter().zip(expected) {
            assert!(
                actual.abs_diff(expected) <= 1,
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
        assert!(!frame.camera.above_native_floor(i).unwrap());
    }
    let encoded = geometry_frames(vec![frame], &|| Ok(())).unwrap();
    assert_eq!(encoded[0].size, [3, 1]);
    assert_eq!(encoded[0].rgb, [[0; 3], [1; 3], [1; 3]]);
}

#[test]
fn retired_geometry_preparation_stops_before_sampling() {
    let frame = captured(1, [0, 0, 1, 1].into(), [1, 1], vec![[1000.; 3]], 1.);
    assert!(geometry_frames(vec![frame], &|| Err("retired".into())).is_err());
}
