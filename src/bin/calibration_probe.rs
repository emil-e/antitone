use std::path::Path;

use antitone::processing::{Calibration, Inversion, RollSettings};
use antitone::raw::{RawFile, Rect};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let gpu_fixture = arguments.get(10).is_some_and(|v| v == "--gpu-fixture");
    if arguments.len() < 10 || (arguments.len() > 10 && !gpu_fixture) {
        return Err("Usage: antitone-calibration-probe DMIN.ARW x y width height DMAX.ARW x y width height [--gpu-fixture additional.ARW ...] (full developed-source coordinates)".into());
    }
    if gpu_fixture && !cfg!(feature = "gpu") {
        return Err("GPU fixtures require --features raw,gpu".into());
    }
    let region = |offset: usize| -> Result<Rect, Box<dyn std::error::Error>> {
        Ok(Rect {
            x: arguments[offset].parse()?,
            y: arguments[offset + 1].parse()?,
            width: arguments[offset + 2].parse()?,
            height: arguments[offset + 3].parse()?,
        })
    };
    let dmin_region = region(1)?;
    let dmax_region = region(6)?;
    let mut dmax_source = RawFile::open(Path::new(&arguments[5]))?;
    let wb = dmax_source.metadata.cam_mul;
    let matrix = dmax_source.metadata.camera_to_ap1()?;
    let floor = antitone::processing::CAMERA_INPUT_FLOOR;
    let dmax_sensor = dmax_source.region_sensor_diagnostics(dmax_region)?;
    let dmax_area = dmax_source.metadata.image_area()?;
    let dmax_image = dmax_source.develop_camera(wb)?;
    let dmax = dmax_image.sample_mean(dmax_region, glam::DMat3::IDENTITY)?;
    #[cfg(feature = "gpu")]
    let mut scans = if gpu_fixture {
        grid(&dmax_image, dmax_area)?
    } else {
        Vec::new()
    };
    #[cfg(not(feature = "gpu"))]
    let _ = dmax_area;
    let mut dmin_source = RawFile::open(Path::new(&arguments[0]))?;
    let dmin_sensor = dmin_source.region_sensor_diagnostics(dmin_region)?;
    let dmin_matrix = dmin_source.metadata.camera_to_ap1()?;
    let dmin_area = dmin_source.metadata.image_area()?;
    let dmin_image = dmin_source.develop_camera(wb)?;
    let dmin = dmin_image.sample_mean(dmin_region, glam::DMat3::IDENTITY)?;
    #[cfg(feature = "gpu")]
    if gpu_fixture {
        if dmin_matrix != matrix {
            return Err("GPU fixture requires one camera conversion matrix".into());
        }
        scans.extend(grid(&dmin_image, dmin_area)?);
    }
    #[cfg(not(feature = "gpu"))]
    let _ = dmin_area;
    println!("Diagnostic candidate regions, not saved/user-selected production calibration.");
    println!("fixed_roll_wb={wb:?} floor={floor:.12e}");
    println!("dmin_region={dmin_region:?} dmin={dmin:?} sensor={dmin_sensor:?}");
    println!("dmax_region={dmax_region:?} dmax={dmax:?} sensor={dmax_sensor:?}");
    if dmin_sensor
        .at_white
        .into_iter()
        .chain(dmax_sensor.at_white)
        .any(|v| v > 0)
    {
        return Err("Reference patch contains sensor values at/above reported linear white".into());
    }
    let calibration = Calibration {
        dmin,
        dmax,
        input_floor: floor,
    }
    .validate()?;
    for gamma in [0.1, 0.65, 1.0, 3.0] {
        let inversion = Inversion::new(
            calibration,
            RollSettings {
                gamma,
                ..RollSettings::default()
            },
        )?;
        let black = inversion.apply(dmin)?.rgb;
        let white = inversion.apply(dmax)?.rgb;
        println!("gamma={gamma} dmin_positive={black:?} dmax_positive={white:?}");
        if black.abs().max_element() > 2e-12
            || (white - glam::DVec3::ONE).abs().max_element() > 2e-12
        {
            return Err("Actual measured-anchor numerical gate failed".into());
        }
    }
    #[cfg(feature = "gpu")]
    if gpu_fixture {
        for path in &arguments[11..] {
            let mut raw = RawFile::open(Path::new(path))?;
            let area = raw.metadata.image_area()?;
            if raw.metadata.camera_to_ap1()? != matrix {
                return Err("GPU fixture requires one camera conversion matrix".into());
            }
            scans.extend(grid(&raw.develop_camera(wb)?, area)?);
        }
        write_gpu_fixture(calibration, matrix, &scans)?;
    }
    Ok(())
}

#[cfg(feature = "gpu")]
fn grid(
    image: &antitone::raw::Developed<'_>,
    area: Rect,
) -> Result<Vec<[u16; 4]>, antitone::Error> {
    let mut result = Vec::with_capacity(1024);
    for y in 0..32 {
        for x in 0..32 {
            result.push(image.pixel_codes(
                area.x + (2 * x + 1) * area.width / 64,
                area.y + (2 * y + 1) * area.height / 64,
            )?);
        }
    }
    Ok(result)
}

#[cfg(feature = "gpu")]
fn write_gpu_fixture(
    calibration: Calibration,
    matrix: glam::DMat3,
    scans: &[[u16; 4]],
) -> Result<(), Box<dyn std::error::Error>> {
    use antitone::color::Balance;
    use antitone::gpu::ProcessingParameters;
    use antitone::processing::{PhotoSettings, Pipeline};
    let roll = RollSettings {
        balance: Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        exposure: 0.5,
        ..Default::default()
    };
    let photo = PhotoSettings {
        exposure: 0.25,
        wb: Balance {
            warmth: -0.25,
            tint: 0.15,
        },
        ..PhotoSettings::default()
    };
    let parameters = ProcessingParameters::positive(calibration, roll, photo, matrix)?;
    let pipeline = Pipeline::new(calibration, roll, photo, matrix)?;
    let mut working = Vec::with_capacity(scans.len());
    let mut flags = Vec::with_capacity(scans.len());
    for &[r, g, b, _] in scans {
        let scan = glam::DVec3::new(r as f64, g as f64, b as f64) / 65535.;
        let pixel = pipeline.apply(scan)?;
        working.push(pixel.rgb.as_vec3().extend(1.0));
        flags.push(
            pixel
                .floored
                .iter()
                .enumerate()
                .fold(0u8, |m, (c, f)| m | (u8::from(*f) << c)),
        );
    }
    std::fs::create_dir_all(".cache/probes")?;
    std::fs::write(
        ".cache/probes/camera-source-model4.rgb16",
        bytemuck::cast_slice(scans),
    )?;
    std::fs::write(
        ".cache/probes/camera-to-ap1-model4.f64",
        bytemuck::cast_slice(&matrix.to_cols_array()),
    )?;
    std::fs::write(
        ".cache/probes/camera-working-ap1-model4.f32",
        bytemuck::cast_slice(&working),
    )?;
    std::fs::write(".cache/probes/camera-floor-flags-model4.u8", flags)?;
    std::fs::write(
        ".cache/probes/camera-processing-model4.bin",
        bytemuck::bytes_of(&parameters),
    )?;
    println!(
        "GPU fixture: {} real camera-developed pixels; roll={roll:?} photo={photo:?}",
        scans.len()
    );
    Ok(())
}
