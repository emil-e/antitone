//! Writes deterministic RGB16/model-5 CPU references for the optional OCIO/Metal camera chain.
//! Requires pinned LibRaw and the committed patch DNG; `gpu` supplies the parameter ABI, not a device.
//! Overwrites five generated `.cache/probes` artifacts. Includes two explicit code-floor cases.
//! These are numerical inputs, not photographic calibration or an independent output oracle.
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    color::Balance,
    geometry::SourceRaster,
    processing::{Calibration, RollSettings},
    raw::{RawFile, Rect},
    scan::Scan,
};
use glam::DVec3;
use std::{fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err(
            "Usage: antitone-camera-reference (authored inputs; writes .cache/probes)".into(),
        );
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let mut scan = Scan::open(&path, None)?;
    let calibration = Calibration {
        dmin: scan.reference(Rect {
            x: 16,
            y: 16,
            width: 32,
            height: 32,
        })?,
        dmax: scan.reference(Rect {
            x: 80,
            y: 16,
            width: 32,
            height: 32,
        })?,
        input_floor: scan.input_floor,
    }
    .validate()?;
    let matrix = scan.camera_to_ap1;
    let mut raw = RawFile::open(&path)?;
    let image = raw.develop_camera(scan.fixed_wb)?;
    let raster = SourceRaster::new(scan.area, [32, 32])?;
    let mut codes = Vec::with_capacity(1026);
    for y in 0..32 {
        for x in 0..32 {
            let [sx, sy] = raster.source_pixel([x, y])?;
            codes.push(image.pixel_codes(sx, sy)?);
        }
    }
    codes.extend([[0, 0, 0, 0], [3, 0, 1, 0]]);
    let input = antitone::processing::InputSettings::Negative {
        dmin: Some(calibration.dmin),
        dmax: Some(calibration.dmax),
        input_floor: calibration.input_floor,
        response: antitone::processing::Response::film(),
        gamma: antitone::processing::DEFAULT_GAMMA,
        balance: Balance {
            warmth: 0.2,
            tint: -0.1,
        },
    };
    let roll = RollSettings {
        exposure: 0.5,
        ..RollSettings::default()
    };
    let photo = AdjustmentLayerSettings {
        exposure: 0.25,
        wb: Balance {
            warmth: -0.25,
            tint: 0.15,
        },
        ..AdjustmentLayerSettings::default()
    };
    let pipeline = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(photo),
    ])
    .and_then(|layers| {
        antitone::adjustment_layers::AdjustmentLayerPipeline::new(input, roll, layers, matrix)
    })?;
    let mut working = Vec::with_capacity(codes.len());
    let mut flags = Vec::with_capacity(codes.len());
    for &[r, g, b, _] in &codes {
        let pixel = pipeline.apply(DVec3::new(r as f64, g as f64, b as f64) / 65535., [0, 0])?;
        working.push(pixel.rgb.as_vec3().extend(1.));
        flags.push(
            pixel
                .floored
                .iter()
                .enumerate()
                .fold(0u8, |mask, (c, f)| mask | (u8::from(*f) << c)),
        );
    }
    fs::create_dir_all(".cache/probes")?;
    fs::write(
        ".cache/probes/camera-source-model5.rgb16",
        bytemuck::cast_slice(&codes),
    )?;
    fs::write(
        ".cache/probes/camera-to-ap1-model5.f64",
        bytemuck::cast_slice(&matrix.to_cols_array()),
    )?;
    fs::write(
        ".cache/probes/camera-working-ap1-model5.f32",
        bytemuck::cast_slice(&working),
    )?;
    fs::write(".cache/probes/camera-floor-flags-model5.u8", flags)?;
    println!("camera_reference authored_pixels=1024 code_floor_cases=2 output=.cache/probes");
    Ok(())
}
