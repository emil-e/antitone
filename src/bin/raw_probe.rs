use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use antitone::processing::{Calibration, Inversion, RollSettings};
use antitone::raw::{Developed, RawFile, Rect, version};
use antitone::sampling::mean;
use glam::{DMat3, DVec3};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    let metadata_only = first.as_deref() == Some(std::ffi::OsStr::new("--metadata"));
    let diagnostic_ppm = first.as_deref() == Some(std::ffi::OsStr::new("--diagnostic-ppm"));
    let paths: Vec<PathBuf> = if metadata_only || diagnostic_ppm {
        arguments.map(PathBuf::from).collect()
    } else {
        first
            .into_iter()
            .chain(arguments)
            .map(PathBuf::from)
            .collect()
    };
    if paths.is_empty() {
        return Err("Usage: antitone-raw-probe [--metadata] FILE.ARW ...".into());
    }
    let roll = RollSettings::default();
    println!(
        "LibRaw {} / processing model {}",
        version(),
        roll.response.processing_model()
    );
    let mut fixed_wb = None;
    for path in paths {
        let start = Instant::now();
        let mut raw = RawFile::open(&path)?;
        let metadata = raw.metadata;
        let wb = *fixed_wb.get_or_insert(metadata.cam_mul);
        let matrix = metadata.camera_to_ap1()?;
        let area = metadata.image_area()?;
        println!("\nSOURCE {}", path.display());
        println!(
            "camera={} {} decoder={} bits={} sony_type={}",
            metadata.make(),
            metadata.model(),
            metadata.decoder(),
            metadata.raw_bps,
            metadata.sony_file_type
        );
        println!(
            "decoded={}x{} metadata_area={area:?} flip={}",
            metadata.width, metadata.height, metadata.flip
        );
        println!(
            "as_shot={:?} fixed_roll_wb={wb:?} daylight={:?}",
            metadata.cam_mul, metadata.daylight_mul
        );
        println!(
            "black={} cblack={:?} maximum={} linear_max={:?}",
            metadata.black, metadata.cblack, metadata.maximum, metadata.linear_max
        );
        println!(
            "camera_to_srgb_columns={:?}",
            metadata.camera_to_srgb().to_cols_array_2d()
        );
        println!("camera_to_ap1_columns={:?}", matrix.to_cols_array_2d());
        if metadata_only {
            continue;
        }
        println!("sensor_full_frame={:?}", raw.unpack()?);
        let developed = raw.develop_camera(wb)?;
        if diagnostic_ppm {
            save_diagnostic_ppm(&path, &developed, area, metadata.camera_to_srgb())?;
        }
        println!(
            "processed_maximum={} data_maximum={} normalized_pre_mul={:?} warnings={:#x}",
            developed.maximum, developed.data_maximum, developed.pre_mul, developed.warnings
        );
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let floor = antitone::processing::CAMERA_INPUT_FLOOR;
        let mut nonpositive = [0u64; 3];
        let mut below_floor = [0u64; 3];
        let mut floored_pixels = 0u64;
        let mut zeros = [0u64; 3];
        let mut saturated = [0u64; 3];
        let mut sum = DVec3::ZERO;
        let mut points = Vec::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let camera = developed.pixel(x, y)?;
                min = min.min(camera);
                max = max.max(camera);
                sum += camera;
                floored_pixels += u64::from(camera.min_element() < floor);
                for c in 0..3 {
                    nonpositive[c] += u64::from(camera[c] <= 0.0);
                    below_floor[c] += u64::from(camera[c] < floor);
                    zeros[c] += u64::from(camera[c] == 0.0);
                    saturated[c] += u64::from(camera[c] == 1.0);
                }
                if (x - area.x) % 32 == 0 && (y - area.y) % 32 == 0 {
                    points.push((x, y, metadata.camera_to_srgb() * camera, camera));
                }
            }
        }
        let count = u64::from(area.width) * u64::from(area.height);
        println!(
            "image_pixels={count} camera_min={min:?} camera_max={max:?} camera_mean={:?}",
            sum / count as f64
        );
        println!(
            "camera_nonpositive={nonpositive:?} camera_zero={zeros:?} camera_u16_ceiling={saturated:?}"
        );
        println!(
            "camera_below_precision_floor={below_floor:?} affected_pixels={floored_pixels}/{count}"
        );
        println!(
            "one_camera_code_floor={floor:.12e} (precision bound, not a measured noise estimate)"
        );
        if below_floor.into_iter().any(|v| v > count / 100) {
            return Err(
                "More than 1% of a channel needs flooring; investigate development/conversion"
                    .into(),
            );
        }

        let mut repeat_error = 0.0_f64;
        let repeated = raw.develop_camera(wb)?;
        for &(x, y, _, camera) in &points {
            repeat_error = repeat_error.max((repeated.pixel(x, y)? - camera).abs().max_element());
        }
        let reference = raw.develop_srgb_reference(wb)?;
        let mut matrix_error = 0.0_f64;
        let mut compared = 0usize;
        let mut outside_srgb = 0usize;
        for &(x, y, expected, _) in &points {
            if expected.min_element() < 0.0 || expected.max_element() > 1.0 {
                outside_srgb += 1;
                continue;
            }
            matrix_error =
                matrix_error.max((reference.pixel(x, y)? - expected).abs().max_element());
            compared += 1;
        }
        println!(
            "repeat_camera_max_error={repeat_error:.12e} compared_grid={compared} outside_srgb_grid={outside_srgb} LibRaw_srgb_matrix_max_error={matrix_error:.12e}"
        );
        if repeat_error != 0.0 || matrix_error > 2.0 / 65535.0 {
            return Err("RAW repeatability or camera-matrix reference gate failed".into());
        }
        let center_scan: Vec<_> = points
            .iter()
            .filter(|(x, y, _, _)| {
                *x > area.x + area.width / 3
                    && *x < area.x + 2 * area.width / 3
                    && *y > area.y + area.height / 3
                    && *y < area.y + 2 * area.height / 3
            })
            .map(|(_, _, _, camera)| *camera)
            .collect();
        println!("central_scan_patch_grid_mean={:?}", mean(&center_scan)?);
        let synthetic_cal = Calibration {
            dmin: DVec3::splat(0.5),
            dmax: DVec3::splat(0.002),
            input_floor: floor,
        };
        let inversion = Inversion::new(synthetic_cal, roll)?;
        let mut affected = 0usize;
        for &(_, _, _, camera) in &points {
            affected += usize::from(inversion.apply(camera)?.floored.iter().any(|v| *v));
        }
        println!(
            "synthetic_anchor_grid_floored={affected}/{} (not real roll calibration)",
            points.len()
        );
        println!("elapsed_total_seconds={:.3}", start.elapsed().as_secs_f64());
    }
    Ok(())
}

fn save_diagnostic_ppm(
    source: &Path,
    developed: &Developed<'_>,
    area: Rect,
    camera_to_srgb: DMat3,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(".cache/probes")?;
    let mut output =
        PathBuf::from(".cache/probes").join(source.file_name().ok_or("missing filename")?);
    output.set_extension("ppm");
    let width = area.width / 8;
    let height = area.height / 8;
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let rgb = camera_to_srgb * developed.pixel(area.x + x * 8, area.y + y * 8)? * 8.0;
            for value in rgb.to_array() {
                let value = value.clamp(0.0, 1.0);
                let encoded = if value <= 0.0031308 {
                    value * 12.92
                } else {
                    1.055 * value.powf(1.0 / 2.4) - 0.055
                };
                bytes.push((encoded * 255.0).round() as u8);
            }
        }
    }
    let mut file = std::fs::File::create(&output)?;
    write!(
        file,
        "P6\n# Diagnostic source only: +3 EV, clipped sRGB, no ACES, no ICC\n{width} {height}\n255\n"
    )?;
    file.write_all(&bytes)?;
    println!(
        "diagnostic_source_ppm={} (+3 EV, clipped sRGB, no ACES/ICC; not managed application output)",
        output.display()
    );
    Ok(())
}
