//! Reference inspection only; rectangle detection here is a prototype, not production policy.
#[path = "../flat_field.rs"]
mod field;
use antitone::{
    color::AP0_TO_XYZ,
    raw::{RawFile, Rect},
};
use glam::{DMat3, DVec3};
use std::{fs, io::Write, path::PathBuf};

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn support(profile: &[f64], threshold: f64) -> (usize, usize) {
    let center = profile.len() / 2;
    assert!(
        profile[center] > threshold,
        "central illumination is not usable"
    );
    let mut first = center;
    let mut end = center + 1;
    while first > 0 && profile[first - 1] > threshold {
        first -= 1;
    }
    while end < profile.len() && profile[end] > threshold {
        end += 1;
    }
    // Discard boundary-containing cells before estimating a field.
    (first + 1, end - 1)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangular_support_proposal_follows_different_openings_not_sample_coordinates() {
        for (length, left, right) in [(101, 3, 96), (81, 14, 66), (61, 1, 60)] {
            let profile: Vec<_> = (0..length)
                .map(|x| {
                    if x < left || x >= right {
                        0.01
                    } else {
                        1. - 0.4 * ((x as f64 - length as f64 / 2.) / (length as f64 / 2.)).powi(2)
                    }
                })
                .collect();
            assert_eq!(support(&profile, 0.25), (left + 1, right - 1));
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err("Usage: flat-field-inspect REFERENCE.ARW ...".into());
    }
    let output = PathBuf::from(".cache/probes/flat-field-inspection");
    fs::create_dir_all(&output)?;
    let mut fixed_wb = None;
    for path in paths {
        let started = std::time::Instant::now();
        let mut raw = RawFile::open(&path)?;
        let wb = *fixed_wb.get_or_insert(raw.metadata.cam_mul);
        let matrix = raw.metadata.camera_to_ap0()?;
        let area = raw.metadata.image_area()?;
        let side = area.width.max(area.height).div_ceil(256);
        let width = area.width.div_ceil(side) as usize;
        let height = area.height.div_ceil(side) as usize;
        let center = Rect {
            x: area.x + area.width / 2 - 128,
            y: area.y + area.height / 2 - 128,
            width: 256,
            height: 256,
        };
        let mut sums = vec![[0u64; 3]; width * height];
        let mut counts = vec![0u64; width * height];
        let center_camera;
        {
            let image = raw.develop_camera(wb)?;
            center_camera = image.sample_mean(center, DMat3::IDENTITY)?;
            for y in 0..area.height {
                for x in 0..area.width {
                    let index = (y / side) as usize * width + (x / side) as usize;
                    let codes = image.pixel_codes(area.x + x, area.y + y)?;
                    for channel in 0..3 {
                        sums[index][channel] += u64::from(codes[channel]);
                    }
                    counts[index] += 1;
                }
            }
        }
        let camera: Vec<_> = sums
            .iter()
            .zip(&counts)
            .map(|(sum, n)| {
                DVec3::new(sum[0] as f64, sum[1] as f64, sum[2] as f64) / (*n as f64 * 65535.)
            })
            .collect();
        let luminance: Vec<_> = camera
            .iter()
            .map(|rgb| (AP0_TO_XYZ * (matrix * *rgb)).y)
            .collect();
        let center_y = (AP0_TO_XYZ * (matrix * center_camera)).y;
        let columns: Vec<_> = (0..width)
            .map(|x| {
                median(
                    (height / 4..3 * height / 4)
                        .map(|y| luminance[y * width + x])
                        .collect(),
                )
            })
            .collect();
        let rows: Vec<_> = (0..height)
            .map(|y| median(luminance[y * width + width / 4..y * width + 3 * width / 4].to_vec()))
            .collect();
        let (left, right) = support(&columns, center_y * 0.25);
        let (top, bottom) = support(&rows, center_y * 0.25);
        if left >= right || top >= bottom {
            return Err("rectangular support is too small".into());
        }
        let rect = Rect {
            x: area.x + left as u32 * side,
            y: area.y + top as u32 * side,
            width: (right - left) as u32 * side,
            height: (bottom - top) as u32 * side,
        };
        let diagnostics = raw.region_sensor_diagnostics(rect)?;
        let mut ev_min = f64::INFINITY;
        let mut ev_max = f64::NEG_INFINITY;
        let mut channel_residual_min = DVec3::splat(f64::INFINITY);
        let mut channel_residual_max = DVec3::splat(f64::NEG_INFINITY);
        for y in top..bottom {
            for x in left..right {
                let i = y * width + x;
                let gain = center_y / luminance[i];
                let residual = ((camera[i] / center_camera) * gain).map(f64::log2);
                ev_min = ev_min.min(gain.log2());
                ev_max = ev_max.max(gain.log2());
                channel_residual_min = channel_residual_min.min(residual);
                channel_residual_max = channel_residual_max.max(residual);
            }
        }
        let target_camera = matrix.inverse() * DVec3::splat(center_y);
        assert!(target_camera.min_element() > 0.);
        println!(
            "neutral_AP0_target={center_y:.12} camera_target={target_camera:?} center_gains={:?}",
            target_camera / center_camera
        );
        for degree in [2, 4, 6] {
            let mut fits = Vec::new();
            for channel in 0..3 {
                let mut samples = Vec::new();
                for y in top..bottom {
                    for x in left..right {
                        if (x + y) % 2 != 0 {
                            continue;
                        }
                        let xx = ((x as f64 + 0.5) * side as f64 - area.width as f64 / 2.)
                            / (area.width as f64 / 2.);
                        let yy = ((y as f64 + 0.5) * side as f64 - area.height as f64 / 2.)
                            / (area.height as f64 / 2.);
                        samples.push([
                            xx,
                            yy,
                            (camera[y * width + x][channel] / center_camera[channel]).ln(),
                        ]);
                    }
                }
                fits.push(field::Surface::fit(&samples, degree));
            }
            let mut errors = Vec::new();
            let mut edges = Vec::new();
            let mut worst = Vec::new();
            let mut gain_min = DVec3::splat(f64::INFINITY);
            let mut gain_max = DVec3::splat(f64::NEG_INFINITY);
            for y in top..bottom {
                for x in left..right {
                    let xx = ((x as f64 + 0.5) * side as f64 - area.width as f64 / 2.)
                        / (area.width as f64 / 2.);
                    let yy = ((y as f64 + 0.5) * side as f64 - area.height as f64 / 2.)
                        / (area.height as f64 / 2.);
                    let relative = DVec3::new(
                        fits[0].log_relative_light(xx, yy).exp(),
                        fits[1].log_relative_light(xx, yy).exp(),
                        fits[2].log_relative_light(xx, yy).exp(),
                    );
                    let gain = target_camera / (center_camera * relative);
                    gain_min = gain_min.min(gain);
                    gain_max = gain_max.max(gain);
                    if (x + y) % 2 == 0 {
                        continue;
                    }
                    let corrected = matrix * (camera[y * width + x] * gain);
                    let error = (corrected / DVec3::splat(center_y)).map(f64::log2).abs();
                    errors.push(error);
                    worst.push((
                        error.max_element(),
                        area.x as f64 + (x as f64 + 0.5) * side as f64,
                        area.y as f64 + (y as f64 + 0.5) * side as f64,
                    ));
                    if x < left + 3 || x >= right - 3 || y < top + 3 || y >= bottom - 3 {
                        edges.push(error);
                    }
                }
            }
            worst.sort_by(|a, b| b.0.total_cmp(&a.0));
            if degree == 6 {
                println!("worst_held_out_degree6_error_EV_x_y={:?}", &worst[..8]);
            }
            let rms = (errors.iter().map(|e| e.length_squared()).sum::<f64>()
                / (3 * errors.len()) as f64)
                .sqrt();
            let mut sorted: Vec<_> = errors.iter().flat_map(|e| e.to_array()).collect();
            sorted.sort_by(f64::total_cmp);
            let edge_max = edges.iter().map(|e| e.max_element()).fold(0.0, f64::max);
            println!(
                "robust_log_polynomial_degree={degree} held_out_blocks={} neutral_RGB_EV_rms={rms:.8} abs_p99={:.8} max={:.8} edge_max={edge_max:.8} gain_min={gain_min:?} max={gain_max:?}",
                errors.len(),
                sorted[sorted.len() * 99 / 100],
                sorted.last().unwrap()
            );
        }
        for sigma in [3., 6., 12.] {
            let (w, h) = (right - left, bottom - top);
            let fits: Vec<_> = (0..3)
                .map(|channel| {
                    let logs: Vec<_> = (top..bottom)
                        .flat_map(|y| {
                            (left..right).map({
                                let camera = &camera;
                                move |x| {
                                    (camera[y * width + x][channel] / center_camera[channel]).ln()
                                }
                            })
                        })
                        .collect();
                    field::LocalSurface::fit(&logs, w, h, sigma)
                })
                .collect();
            let cx = area.width as f64 / (2. * side as f64) - 0.5 - left as f64;
            let cy = area.height as f64 / (2. * side as f64) - 0.5 - top as f64;
            let anchors = DVec3::new(
                fits[0].sample(cx, cy),
                fits[1].sample(cx, cy),
                fits[2].sample(cx, cy),
            );
            let mut errors = Vec::new();
            let mut edge_errors = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    if (x + y) % 2 == 0 {
                        continue;
                    }
                    let relative =
                        (DVec3::new(fits[0].at(x, y), fits[1].at(x, y), fits[2].at(x, y))
                            - anchors)
                            .map(f64::exp);
                    let corrected = matrix
                        * (camera[(y + top) * width + x + left]
                            * (target_camera / (center_camera * relative)));
                    let e = (corrected / DVec3::splat(center_y)).map(f64::log2).abs();
                    errors.push(e);
                    if x < 3 || x >= w - 3 || y < 3 || y >= h - 3 {
                        edge_errors.push(e);
                    }
                }
            }
            let rms = (errors.iter().map(|e| e.length_squared()).sum::<f64>()
                / (3 * errors.len()) as f64)
                .sqrt();
            let mut sorted: Vec<_> = errors.iter().flat_map(|e| e.to_array()).collect();
            sorted.sort_by(f64::total_cmp);
            let mut edge_sorted: Vec<_> = edge_errors.iter().flat_map(|e| e.to_array()).collect();
            edge_sorted.sort_by(f64::total_cmp);
            println!(
                "local_quadratic_gaussian_sigma_cells={sigma} held_out_neutral_RGB_EV_rms={rms:.8} abs_p99={:.8} max={:.8} edge_p99={:.8}",
                sorted[sorted.len() * 99 / 100],
                sorted.last().unwrap(),
                edge_sorted[edge_sorted.len() * 99 / 100]
            );
            if sigma == 6. {
                let image = raw.camera_image()?;
                let mut patch_errors = Vec::new();
                for iy in 0..11 {
                    for ix in 0..17 {
                        let px = rect.x + 34 + ((rect.width - 132) as u64 * ix / 16) as u32;
                        let py = rect.y + 34 + ((rect.height - 132) as u64 * iy / 10) as u32;
                        let mut sum = DVec3::ZERO;
                        for y in py..py + 64 {
                            for x in px..px + 64 {
                                let xx = (x as f64 + 0.5 - area.x as f64) / side as f64
                                    - 0.5
                                    - left as f64;
                                let yy = (y as f64 + 0.5 - area.y as f64) / side as f64
                                    - 0.5
                                    - top as f64;
                                let relative = (DVec3::new(
                                    fits[0].sample(xx, yy),
                                    fits[1].sample(xx, yy),
                                    fits[2].sample(xx, yy),
                                ) - anchors)
                                    .map(f64::exp);
                                sum += matrix
                                    * (image.pixel(x, y)?
                                        * (target_camera / (center_camera * relative)));
                            }
                        }
                        patch_errors.push((sum / (4096. * center_y)).map(f64::log2).abs());
                    }
                }
                let rms = (patch_errors.iter().map(|e| e.length_squared()).sum::<f64>()
                    / (3 * patch_errors.len()) as f64)
                    .sqrt();
                let max = patch_errors
                    .iter()
                    .map(|e| e.max_element())
                    .fold(0., f64::max);
                println!(
                    "independent_full_resolution_64x64_patches={} pixels={} neutral_RGB_EV_rms={rms:.8} max={max:.8}",
                    patch_errors.len(),
                    patch_errors.len() * 4096
                );
            }
        }
        let means: Vec<_> = (top..bottom)
            .flat_map(|y| {
                (left..right).map({
                    let camera = &camera;
                    move |x| camera[y * width + x]
                })
            })
            .collect();
        let flat = antitone::flat_field::FlatField::estimate(
            [rect.x, rect.y, rect.width, rect.height],
            [right - left, bottom - top],
            &means,
            matrix,
            center_camera,
            [
                area.x as f64 + area.width as f64 / 2.,
                area.y as f64 + area.height as f64 / 2.,
            ],
        )?;
        let image = raw.camera_image()?;
        let mut patch_errors = Vec::new();
        for iy in 0..11 {
            for ix in 0..17 {
                let px = rect.x + ((rect.width - 64) as u64 * ix / 16) as u32;
                let py = rect.y + ((rect.height - 64) as u64 * iy / 10) as u32;
                let mut sum = DVec3::ZERO;
                for y in py..py + 64 {
                    for x in px..px + 64 {
                        sum += matrix
                            * (image.pixel(x, y)? * flat.gain([x as f64 + 0.5, y as f64 + 0.5])?);
                    }
                }
                patch_errors.push((sum / (4096. * center_y)).map(f64::log2).abs());
            }
        }
        let rms = (patch_errors.iter().map(|e| e.length_squared()).sum::<f64>()
            / (3 * patch_errors.len()) as f64)
            .sqrt();
        let max = patch_errors
            .iter()
            .map(|e| e.max_element())
            .fold(0., f64::max);
        let mut corrected_center = DVec3::ZERO;
        for y in center.y..center.y + center.height {
            for x in center.x..center.x + center.width {
                corrected_center +=
                    matrix * (image.pixel(x, y)? * flat.gain([x as f64 + 0.5, y as f64 + 0.5])?);
            }
        }
        corrected_center /= (center.width * center.height) as f64;
        println!(
            "scalar_field contract={} nodes={} source_edge_64x64_patches={} neutral_RGB_EV_rms={rms:.8} max={max:.8} center_neutral_RGB_EV={:?} center_luminance_EV={:.8} gain_bounds={:?}",
            antitone::flat_field::CONTRACT,
            flat.nodes().len(),
            patch_errors.len(),
            (corrected_center / DVec3::splat(center_y)).map(f64::log2),
            ((AP0_TO_XYZ * corrected_center).y / center_y).log2(),
            flat.bounds()
        );
        let name = path.file_stem().unwrap().to_string_lossy();
        let mut csv = fs::File::create(output.join(format!("{name}-blocks.csv")))?;
        writeln!(csv, "x,y,r,g,b,Y")?;
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let px = area.x as f64
                    + (x as u32 * side) as f64
                    + ((area.width - x as u32 * side).min(side) as f64) / 2.;
                let py = area.y as f64
                    + (y as u32 * side) as f64
                    + ((area.height - y as u32 * side).min(side) as f64) / 2.;
                writeln!(
                    csv,
                    "{px},{py},{:.12},{:.12},{:.12},{:.12}",
                    camera[i].x, camera[i].y, camera[i].z, luminance[i]
                )?;
            }
        }
        println!(
            "source={} LibRaw={} fixed_wb={wb:?} area={area:?} grid={width}x{height} block_side={side}",
            path.display(),
            antitone::raw::version()
        );
        println!("center_patch={center:?} center_camera={center_camera:?} center_Y={center_y:.12}");
        println!(
            "PROTOTYPE automatic_rect={rect:?} center_relative_dark_threshold=0.25 safety_margin_one_block=true diagnostics={diagnostics:?}"
        );
        println!(
            "unfiltered_block_correction_EV=[{ev_min:.6},{ev_max:.6}] scalar_corrected_channel_residual_EV_min={channel_residual_min:?} max={channel_residual_max:?}"
        );
        println!(
            "output={} seconds={:.3}",
            output.join(format!("{name}-blocks.csv")).display(),
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
