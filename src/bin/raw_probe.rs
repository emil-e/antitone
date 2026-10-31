//! Read-only RAW diagnostic; metadata-only mode uses the explicit LibRaw baseline.
//! Full mode gates sampled repeatability and eligible in-sRGB matrix comparisons.
//! Zero eligible comparisons does not fail; inspect the reported comparison count.
//! Optional `.cache/probes` PPMs are clipped unmanaged views, not grades or photographic acceptance.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use antitone::processing::{Calibration, Inversion, NegativeInput, Response};
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
        return Err("Usage: antitone-raw-probe [--metadata|--diagnostic-ppm] FILE.ARW ...".into());
    }
    let ppm_ev = std::env::var("ANTITONE_RAW_PPM_EV")
        .ok()
        .map(|value| value.parse::<f64>())
        .transpose()?
        .unwrap_or(3.);
    let ppm_gain = ppm_ev.exp2();
    if !ppm_ev.is_finite() || !ppm_gain.is_finite() || ppm_gain <= 0. {
        return Err("invalid diagnostic PPM exposure".into());
    }
    println!(
        "LibRaw {} / processing model {}",
        version(),
        antitone::PROCESSING_MODEL_VERSION
    );
    let mut fixed_wb = None;
    for path in paths {
        let start = Instant::now();
        let mut raw = if metadata_only {
            RawFile::open_uncorrected(&path)?
        } else {
            RawFile::open(&path)?
        };
        let metadata = raw.metadata;
        let wb = *fixed_wb.get_or_insert_with(|| metadata.as_shot_wb());
        let matrix = metadata.camera_to_ap1()?;
        let area = metadata.image_area()?;
        println!("\nSOURCE {}", path.display());
        println!(
            "camera={} {} decoder={} bits={}",
            metadata.make(),
            metadata.model(),
            metadata.decoder(),
            metadata.raw_bps
        );
        if metadata.make() == "Sony" {
            println!(
                "vendor=Sony sony_file_type={}",
                metadata.vendor.sony_file_type
            );
        } else {
            println!("vendor=none");
        }
        println!(
            "decoded={}x{} metadata_area={area:?} flip={} photo_orientation={:?}",
            metadata.width,
            metadata.height,
            metadata.flip,
            metadata.photo_orientation()?
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
        println!(
            "embedded_lens_distortion={:?} embedded_lens_ca={:?} embedded_lens_vignetting={:?}",
            metadata.vendor.sony_lens.distortion_status(),
            metadata.vendor.sony_lens.ca_status(),
            metadata.vendor.sony_lens.vignetting_status()
        );
        match metadata.vendor.sony_lens.profile() {
            Ok(Some(profile)) => {
                println!(
                    "embedded_lens_knots={} distortion={} ca={} inverse_scales_at_unit_radius={:?} vignetting={} maximum_gain={:.9}",
                    profile.knot_count(),
                    profile.has_distortion(),
                    profile.has_ca(),
                    profile.inverse_scales(1., antitone::lens::LensCorrections::ALL)?,
                    profile.has_vignetting(),
                    profile.maximum_gain()
                );
                if profile.has_distortion() || profile.has_ca() || profile.has_vignetting() {
                    let geometry = antitone::lens::EmbeddedLensGeometry::new(
                        profile,
                        metadata.native_image_area()?,
                    )?;
                    println!(
                        "embedded_lens_automatic_output={:?} pixel_development_requested={}",
                        geometry.size(),
                        !metadata_only
                    );
                }
            }
            Ok(None) => println!("embedded_lens_profile=none"),
            Err(error) => println!("embedded_lens_profile=rejected: {error}"),
        }
        if metadata_only {
            continue;
        }
        println!("sensor_full_frame={:?}", raw.unpack()?);
        let developed = raw.develop_camera(wb)?;
        if diagnostic_ppm {
            save_diagnostic_ppm(
                &path,
                &developed,
                area,
                metadata.camera_to_srgb(),
                if metadata.lens_identity()?.is_some() {
                    "lens-corrected"
                } else {
                    "native"
                },
                ppm_gain,
            )?;
        }
        println!(
            "processed_maximum={} data_maximum={} normalized_pre_mul={:?} warnings={:#x}",
            developed.maximum, developed.data_maximum, developed.pre_mul, developed.warnings
        );
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let floor = metadata.input_floor()?;
        let mut nonpositive = [0u64; 3];
        let mut below_floor = [0u64; 3];
        let mut floored_pixels = 0u64;
        let mut zeros = [0u64; 3];
        let mut at_native_white = [0u64; 3];
        let mut above_native_white = [0u64; 3];
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
                    at_native_white[c] += u64::from(camera[c] == 1.0);
                    above_native_white[c] += u64::from(camera[c] > 1.0);
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
            "camera_nonpositive={nonpositive:?} camera_zero={zeros:?} camera_at_native_white={at_native_white:?} camera_above_native_white={above_native_white:?}"
        );
        println!(
            "camera_below_precision_floor={below_floor:?} affected_pixels={floored_pixels}/{count}"
        );
        println!(
            "developed_camera_floor={floor:.12e} (amplified native quantum, not a measured noise estimate)"
        );

        let mut repeat_error = 0.0_f64;
        let repeated = raw.develop_camera(wb)?;
        for &(x, y, _, camera) in &points {
            repeat_error = repeat_error.max((repeated.pixel(x, y)? - camera).abs().max_element());
        }
        if metadata.dng_identity().is_some() {
            println!(
                "repeat_camera_max_error={repeat_error:.12e} SDK_basis=true LibRaw_srgb_reference=not_applicable"
            );
            if repeat_error != 0.0 {
                return Err("SDK camera repeatability gate failed".into());
            }
        } else {
            // Channel-dependent CA cannot be applied to an already mixed/clipped sRGB reference.
            // Check LibRaw's camera matrix on the native raster independently of the corrected grid.
            let mut reference_raw = if metadata.lens_identity()?.is_some() {
                drop(raw);
                RawFile::open_uncorrected(&path)?
            } else {
                raw
            };
            let matrix_points = if metadata.lens_identity()?.is_some() {
                let native_area = reference_raw.metadata.image_area()?;
                let native = reference_raw.develop_camera(wb)?;
                if diagnostic_ppm {
                    save_diagnostic_ppm(
                        &path,
                        &native,
                        native_area,
                        metadata.camera_to_srgb(),
                        "native",
                        ppm_gain,
                    )?;
                }
                let mut squared = DVec3::ZERO;
                let mut largest = DVec3::ZERO;
                let trim = [
                    (native_area.width - area.width) / 2,
                    (native_area.height - area.height) / 2,
                ];
                for &(x, y, _, corrected) in &points {
                    let delta = (native
                        .pixel(native_area.x + trim[0] + x, native_area.y + trim[1] + y)?
                        - corrected)
                        * 65535.;
                    squared += delta * delta;
                    largest = largest.max(delta.abs());
                }
                println!(
                    "lens_vs_native_unwarped_code_rms={:?} max={largest:?} (effect size, not a quality oracle)",
                    (squared / points.len() as f64).sqrt()
                );
                let mut native_points = Vec::new();
                for y in (native_area.y..native_area.y + native_area.height).step_by(32) {
                    for x in (native_area.x..native_area.x + native_area.width).step_by(32) {
                        native_points.push((x, y, metadata.camera_to_srgb() * native.pixel(x, y)?));
                    }
                }
                native_points
            } else {
                points
                    .iter()
                    .map(|&(x, y, expected, _)| (x, y, expected))
                    .collect()
            };
            let reference = reference_raw.develop_srgb_reference(wb)?;
            let mut matrix_error = 0.0_f64;
            let mut compared = 0usize;
            let mut outside_srgb = 0usize;
            for &(x, y, expected) in &matrix_points {
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
        let inversion = Inversion::new(NegativeInput {
            calibration: synthetic_cal,
            response: Response::film(),
            gamma: antitone::processing::DEFAULT_GAMMA,
            balance: antitone::color::Balance::default(),
        })?;
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
    label: &str,
    gain: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(".cache/probes")?;
    let mut output =
        PathBuf::from(".cache/probes").join(source.file_name().ok_or("missing filename")?);
    output.set_extension(format!("{label}.ppm"));
    let width = area.width / 8;
    let height = area.height / 8;
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let rgb = camera_to_srgb * developed.pixel(area.x + x * 8, area.y + y * 8)? * gain;
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
        "P6\n# Diagnostic source only: gain={gain}, clipped sRGB, no ACES, no ICC\n{width} {height}\n255\n"
    )?;
    file.write_all(&bytes)?;
    println!(
        "diagnostic_source_ppm={} (gain={gain}, clipped sRGB, no ACES/ICC; not managed application output)",
        output.display()
    );
    Ok(())
}
