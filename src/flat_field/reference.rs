//! Measure illumination support and camera-RGB gains from uncorrected developed pixels.
use super::{FlatField, propose_support};
use crate::{Error, color::AP1_TO_XYZ, geometry::SourceRegion, scan::DevelopedSource};
use glam::DVec3;

/// Suggest central illuminated support without assuming that it can be measured.
pub fn suggest_region(source: &DevelopedSource) -> Result<Option<SourceRegion>, Error> {
    let area = source.area();
    let side = area.width.max(area.height).div_ceil(256);
    let size = [
        area.width.div_ceil(side) as usize,
        area.height.div_ceil(side) as usize,
    ];
    let image = source.camera_image()?;
    let mut sums = vec![[0f64; 3]; size[0] * size[1]];
    let mut counts = vec![0u64; sums.len()];
    for y in 0..area.height {
        for x in 0..area.width {
            let i = (y / side) as usize * size[0] + (x / side) as usize;
            let pixel = image.pixel_samples(area.x + x, area.y + y)?;
            for c in 0..3 {
                sums[i][c] += f64::from(pixel[c]);
            }
            counts[i] += 1;
        }
    }

    let camera_to_ap1 = source.camera_to_ap1();
    let values: Vec<_> = sums
        .iter()
        .zip(counts)
        .map(|(sum, count)| {
            let camera = DVec3::from_array(*sum) / (count as f64 * 65535.);
            (AP1_TO_XYZ * (camera_to_ap1 * camera)).y
        })
        .collect();
    let center = values[(size[1] / 2) * size[0] + size[0] / 2];
    Ok(
        propose_support(&values, size, center).map(|[left, top, right, bottom]| SourceRegion {
            x: area.x + left as u32 * side,
            y: area.y + top as u32 * side,
            width: (right - left) as u32 * side,
            height: (bottom - top) as u32 * side,
        }),
    )
}

/// Fit gains inside the accepted support from area-weighted original camera samples.
/// The gain is anchored at the developed image centre, not the support centre.
pub fn estimate(source: &DevelopedSource, region: SourceRegion) -> Result<FlatField, Error> {
    let area = source.area();
    if !area.contains_region(region) {
        return Err(Error::InvalidParameter(
            "flat-field support is outside the active source area",
        ));
    }
    let maximum = region.width.max(region.height) as f64;
    let size = [
        (region.width as f64 / maximum * 256.).round() as usize,
        (region.height as f64 / maximum * 256.).round() as usize,
    ];
    if size.iter().any(|n| *n < 7) {
        return Err(Error::UnusableSample("flat-field support is too narrow"));
    }

    let image = source.camera_image()?;
    let mut means = Vec::with_capacity(size[0] * size[1]);
    for gy in 0..size[1] {
        for gx in 0..size[0] {
            let left = region.x as f64 + gx as f64 * region.width as f64 / size[0] as f64;
            let right = region.x as f64 + (gx + 1) as f64 * region.width as f64 / size[0] as f64;
            let top = region.y as f64 + gy as f64 * region.height as f64 / size[1] as f64;
            let bottom = region.y as f64 + (gy + 1) as f64 * region.height as f64 / size[1] as f64;
            let mut sum = DVec3::ZERO;
            for y in top.floor() as u32..bottom.ceil() as u32 {
                for x in left.floor() as u32..right.ceil() as u32 {
                    let weight = (right.min(x as f64 + 1.) - left.max(x as f64))
                        * (bottom.min(y as f64 + 1.) - top.max(y as f64));
                    let pixel = image.pixel_samples(x, y)?;
                    sum += DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) * weight;
                }
            }
            means.push(sum / ((right - left) * (bottom - top) * 65535.));
        }
    }
    FlatField::estimate(
        [region.x, region.y, region.width, region.height],
        size,
        &means,
        [
            area.x as f64 + area.width as f64 / 2.,
            area.y as f64 + area.height as f64 / 2.,
        ],
    )
}
