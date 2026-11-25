//! Original-cell crops and context metrics shared by donor search and fixed-donor healing.

use super::{
    Ap1, BaseImage, CONTEXT_PADDING, Execution, SupportMask, base_image, dilate, distance,
};
use crate::{Error, geometry::SourceRegion};

#[allow(dead_code)]
pub(crate) struct SearchTarget {
    pub(crate) has_exterior: bool,
    pub(crate) origin: [u32; 2],
    pub(crate) size: [usize; 2],
    pub(crate) image: Vec<Ap1>,
    pub(crate) local: Vec<bool>,
    pub(crate) required: Vec<bool>,
    pub(super) known: Vec<bool>,
    pub(super) valid_target: Vec<bool>,
    pub(super) weights: Vec<f64>,
    pub(super) gradients: Vec<[f32; 6]>,
    pub(super) center: [f64; 3],
    pub(super) contrast: f64,
    pub(super) gradient_scale: f64,
}

pub(crate) struct TargetLayout {
    pub(crate) region: SourceRegion,
    has_exterior: bool,
}

pub(crate) fn target_region(group: &SupportMask, active: SourceRegion) -> Option<TargetLayout> {
    let points = group.selected_points().collect::<Vec<_>>();
    let min_x = points.iter().map(|p| p[0]).min()?;
    let max_x = points.iter().map(|p| p[0]).max()?;
    let min_y = points.iter().map(|p| p[1]).min()?;
    let max_y = points.iter().map(|p| p[1]).max()?;
    let x = min_x.saturating_sub(CONTEXT_PADDING).max(active.x);
    let y = min_y.saturating_sub(CONTEXT_PADDING).max(active.y);
    let right = (max_x + CONTEXT_PADDING + 1).min(active.x + active.width);
    let bottom = (max_y + CONTEXT_PADDING + 1).min(active.y + active.height);
    Some(TargetLayout {
        region: SourceRegion {
            x,
            y,
            width: right - x,
            height: bottom - y,
        },
        has_exterior: min_x - x >= 2 && min_y - y >= 2 && right - max_x >= 3 && bottom - max_y >= 3,
    })
}

pub(crate) fn fetch_crop(
    base: &mut impl BaseImage,
    origin: [u32; 2],
    size: [usize; 2],
    offset: [i32; 2],
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<Ap1>>, Error> {
    let region = SourceRegion {
        x: (origin[0] as i32 + offset[0]) as u32,
        y: (origin[1] as i32 + offset[1]) as u32,
        width: size[0] as u32,
        height: size[1] as u32,
    };

    base_image::read(base, region, None, execution)
}

/// One target crop for the best-effort donor search. The donor is scored from
/// its own small crop, so candidate distance never changes the fetch size.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) fn build_search_target(
    support: &SupportMask,
    group: &SupportMask,
    active: SourceRegion,
    base: &mut impl BaseImage,
    execution: &mut Execution<'_>,
) -> Result<Option<SearchTarget>, Error> {
    let Some(layout) = target_region(group, active) else {
        return Ok(None);
    };
    let region = layout.region;
    let (x0, y0) = (region.x, region.y);
    let (width, height) = (region.width as usize, region.height as usize);
    let mut known = vec![false; width * height];
    let mut local = vec![false; width * height];
    let mut mask = vec![false; width * height];
    for y in 0..height {
        if (execution.cancelled)() {
            return Ok(None);
        }
        for x in 0..width {
            let point = [x0 + x as u32, y0 + y as u32];
            let index = y * width + x;
            let masked = support.contains(point);
            mask[index] = masked;
            local[index] = group.contains(point);
            known[index] = !masked;
        }
    }

    let Some(image) = base_image::read(
        base,
        SourceRegion {
            x: x0,
            y: y0,
            width: width as u32,
            height: height as u32,
        },
        Some(&known),
        execution,
    )?
    else {
        return Ok(None);
    };

    let required = dilate(&local, width, height, 2);
    for index in 0..local.len() {
        if required[index] && mask[index] && !local[index] {
            return Err(Error::InvalidParameter(
                "dependent repair masks were not grouped",
            ));
        }
    }
    let known_count = known.iter().filter(|bit| **bit).count();
    if known_count == 0 {
        return Ok(None);
    }
    let Some(smooth) = gaussian_blur(&image, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let Some(gradients) = sobel(&smooth, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let gradient_known = dilate(&mask, width, height, 4)
        .into_iter()
        .map(|unknown| !unknown)
        .collect::<Vec<_>>();
    let valid_target = (0..local.len())
        .map(|index| gradient_known[index])
        .collect::<Vec<_>>();
    let mut center = [0.; 3];
    for index in 0..local.len() {
        if known[index] {
            for channel in 0..3 {
                center[channel] += f64::from(image[index][channel]);
            }
        }
    }
    for value in &mut center {
        *value /= known_count as f64;
    }
    let mut variance = 0.;
    for index in 0..local.len() {
        if known[index] {
            for channel in 0..3 {
                variance += (f64::from(image[index][channel]) - center[channel]).powi(2);
            }
        }
    }
    let center_max = center.iter().map(|value| value.abs()).fold(0., f64::max);
    let contrast = (variance / (known_count * 3) as f64)
        .sqrt()
        .max(center_max * 1e-6)
        .max(1e-12);
    let Some(distances) = distance_to_unknown(&known, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let mut weights = vec![0.; local.len()];
    for index in 0..local.len() {
        if known[index] {
            weights[index] = 0.15 + 0.85 * (-(distances[index] - 1.).max(0.) / 3.).exp();
        }
    }
    let mut gradient_square = 0.;
    let mut gradient_values = 0;
    for index in 0..local.len() {
        if valid_target[index] {
            for channel in gradients[index] {
                gradient_square += f64::from(channel).powi(2);
                gradient_values += 1;
            }
        }
    }
    let gradient_scale = if gradient_values == 0 {
        1.
    } else {
        (gradient_square / gradient_values as f64)
            .sqrt()
            .max(contrast * 1e-6)
    };
    Ok(Some(SearchTarget {
        has_exterior: layout.has_exterior,
        origin: [x0, y0],
        size: [width, height],
        image,
        local,
        required,
        known,
        valid_target,
        weights,
        gradients,
        center,
        contrast,
        gradient_scale,
    }))
}

pub(crate) struct NearSupport {
    origin: [i32; 2],
    size: [usize; 2],
    bits: Vec<bool>,
}

impl NearSupport {
    pub(crate) fn new(support: &SupportMask) -> Self {
        let width = support.size[0] as usize + 8;
        let height = support.size[1] as usize + 8;
        let mut bits = vec![false; width * height];
        for [x, y] in support.selected_points() {
            let local_x = (x - support.origin[0]) as usize + 4;
            let local_y = (y - support.origin[1]) as usize + 4;
            bits[local_y * width + local_x] = true;
        }
        let bits = dilate(&bits, width, height, 4);
        Self {
            origin: [support.origin[0] as i32 - 4, support.origin[1] as i32 - 4],
            size: [width, height],
            bits,
        }
    }

    pub(super) fn contains(&self, point: [i32; 2]) -> bool {
        let x = point[0] - self.origin[0];
        let y = point[1] - self.origin[1];
        x >= 0
            && y >= 0
            && (x as usize) < self.size[0]
            && (y as usize) < self.size[1]
            && self.bits[y as usize * self.size[0] + x as usize]
    }
}

#[allow(clippy::needless_range_loop)]
fn crop_errors(
    target: &SearchTarget,
    support: &SupportMask,
    near: &NearSupport,
    offset: [i32; 2],
    donor: &[Ap1],
    donor_gradients: &[[f32; 6]],
) -> (f64, f64, f64, f64) {
    let [width, _height] = target.size;
    let mut color_weight = 0.;
    let mut color_error = 0.;
    let mut gradient_weight = 0.;
    let mut gradient_error = 0.;
    for index in 0..target.weights.len() {
        let local_x = index % width;
        let local_y = index / width;
        let donor_point = [
            target.origin[0] as i32 + local_x as i32 + offset[0],
            target.origin[1] as i32 + local_y as i32 + offset[1],
        ];
        let weight = target.weights[index];
        let donor_is_support = donor_point[0] >= 0
            && donor_point[1] >= 0
            && support.contains([donor_point[0] as u32, donor_point[1] as u32]);
        if target.known[index] && !donor_is_support {
            color_weight += weight;
            for channel in 0..3 {
                let a = (f64::from(target.image[index][channel]) - target.center[channel])
                    / target.contrast;
                let b =
                    (f64::from(donor[index][channel]) - target.center[channel]) / target.contrast;
                color_error += (a - b).powi(2) * weight;
            }
        }
        if target.valid_target[index] && !near.contains(donor_point) {
            gradient_weight += weight;
            for channel in 0..6 {
                let delta = (f64::from(target.gradients[index][channel])
                    - f64::from(donor_gradients[index][channel]))
                    / target.gradient_scale;
                gradient_error += delta.powi(2) * weight;
            }
        }
    }
    (color_error, color_weight, gradient_error, gradient_weight)
}

#[derive(Clone, Debug, PartialEq)]
pub struct DonorMatch {
    pub target: [u32; 4],
    pub source: [u32; 2],
    pub context_mse: f64,
    pub paired_context_pixels: usize,
    pub matching_score: f64,
}

pub(crate) fn describe_donor(
    target: &SearchTarget,
    support: &SupportMask,
    near: &NearSupport,
    offset: [i32; 2],
    donor: &[Ap1],
    execution: &mut Execution<'_>,
) -> Option<DonorMatch> {
    let [width, height] = target.size;
    let smooth = gaussian_blur(donor, width, height, execution.cancelled)?;
    let donor_gradients = sobel(&smooth, width, height, execution.cancelled)?;
    let (color_error, color_weight, gradient_error, gradient_weight) =
        crop_errors(target, support, near, offset, donor, &donor_gradients);
    let color = if color_weight > 0. {
        0.75 * color_error / (color_weight * 3.)
    } else {
        0.
    };
    let gradient = if gradient_weight > 0. {
        0.25 * gradient_error / (gradient_weight * 6.)
    } else {
        0.
    };
    let matching_score = if color_weight > 0. || gradient_weight > 0. {
        color + gradient
    } else {
        f64::INFINITY
    };
    let mut paired_context_pixels = 0usize;
    let mut context_error = 0.;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let donor_point = [
                target.origin[0] as i32 + x as i32 + offset[0],
                target.origin[1] as i32 + y as i32 + offset[1],
            ];
            let donor_is_support = donor_point[0] >= 0
                && donor_point[1] >= 0
                && support.contains([donor_point[0] as u32, donor_point[1] as u32]);
            if target.known[index] && !donor_is_support {
                paired_context_pixels += 1;
                for (target, donor) in target.image[index].into_iter().zip(donor[index]) {
                    context_error += (f64::from(target) - f64::from(donor)).powi(2);
                }
            }
        }
    }
    Some(DonorMatch {
        target: [
            target.origin[0],
            target.origin[1],
            width as u32,
            height as u32,
        ],
        source: [
            (target.origin[0] as i32 + offset[0]) as u32,
            (target.origin[1] as i32 + offset[1]) as u32,
        ],
        context_mse: if paired_context_pixels > 0 {
            context_error / (paired_context_pixels * 3) as f64
        } else {
            0.
        },
        paired_context_pixels,
        matching_score,
    })
}

fn exact_distance_squares(width: usize, height: usize) -> bool {
    (width.saturating_sub(1) as u64)
        .checked_pow(2)
        .and_then(|x| {
            (height.saturating_sub(1) as u64)
                .checked_pow(2)
                .and_then(|y| x.checked_add(y))
        })
        .is_some_and(|n| n <= 1u64 << 53)
}

pub(super) fn distance_to_unknown(
    known: &[bool],
    width: usize,
    height: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<f64>> {
    // Integer squared distances are identical to the reference's binary64 sums
    // in this range. Apply the same sqrt once to the exact minimum, not once
    // per possible unknown pixel. Keep the scalar path beyond that guarantee.
    let exact = exact_distance_squares(width, height);

    #[derive(Clone, Copy)]
    struct Site {
        base: u64,
        position: u32,
        start: u32,
    }
    if !exact || std::mem::size_of::<Site>() > std::mem::size_of::<(usize, usize)>() {
        return distance_to_unknown_scalar(known, width, cancelled);
    }
    if known.is_empty() {
        return Some(Vec::new());
    }
    let mut values = vec![f64::INFINITY; known.len()];
    for y in 0..height {
        let mut previous = None;
        for x in 0..width {
            let i = y * width + x;
            if i.is_multiple_of(128) && cancelled() {
                return None;
            }
            if !known[i] {
                previous = Some(x);
            }
            if let Some(p) = previous {
                values[i] = (x.abs_diff(p) as u64).pow(2) as f64;
            }
        }
        previous = None;
        for x in (0..width).rev() {
            let i = y * width + x;
            if i.is_multiple_of(128) && cancelled() {
                return None;
            }
            if !known[i] {
                previous = Some(x);
            }
            if let Some(p) = previous {
                values[i] = values[i].min((x.abs_diff(p) as u64).pow(2) as f64);
            }
        }
    }
    // At most one site per row containing an unknown; each site uses no more
    // storage than a scalar-reference unknown coordinate on supported targets.
    let mut envelope: Vec<Site> = Vec::new();
    for x in 0..width {
        envelope.clear();
        for q in 0..height {
            if q.is_multiple_of(128) && cancelled() {
                return None;
            }
            let base = values[q * width + x];
            if !base.is_finite() {
                continue;
            }
            let base = base as u64;
            let start = loop {
                let Some(p) = envelope.last().copied() else {
                    break 0;
                };
                let numerator = (q as i64).pow(2) + base as i64
                    - (i64::from(p.position).pow(2) + p.base as i64);
                // First integer row where the new parabola is strictly lower.
                // Clipping only its active domain preserves equal-distance ties.
                let first = (numerator.div_euclid(2 * (q as i64 - i64::from(p.position))) + 1)
                    .clamp(0, height as i64) as u32;
                if first <= p.start {
                    envelope.pop();
                } else {
                    break first;
                }
            };
            if start < height as u32 {
                envelope.push(Site {
                    base,
                    position: q as u32,
                    start,
                });
            }
        }
        if envelope.is_empty() {
            continue;
        }
        let mut active = 0;
        for y in 0..height {
            if y.is_multiple_of(128) && cancelled() {
                return None;
            }
            while active + 1 < envelope.len() && envelope[active + 1].start <= y as u32 {
                active += 1;
            }
            let p = envelope[active];
            let squared = p.base + (y.abs_diff(p.position as usize) as u64).pow(2);
            values[y * width + x] = (squared as f64).sqrt();
        }
    }
    Some(values)
}

fn distance_to_unknown_scalar(
    known: &[bool],
    width: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<f64>> {
    let unknown = known
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (!value).then_some((index % width, index / width)))
        .collect::<Vec<_>>();
    known
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index % 128 == 0 && cancelled() {
                return None;
            }
            Some(if !value {
                0.
            } else {
                let x = index % width;
                let y = index / width;
                unknown
                    .iter()
                    .map(|(ux, uy)| distance(x, y, *ux, *uy))
                    .fold(f64::INFINITY, f64::min)
            })
        })
        .collect()
}

pub(super) fn gaussian_blur(
    image: &[Ap1],
    width: usize,
    height: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Ap1>> {
    let mut kernel = [0.; 7];
    for (index, value) in kernel.iter_mut().enumerate() {
        *value = (-0.5 * (index as f64 - 3.).powi(2)).exp();
    }
    let sum = kernel.iter().sum::<f64>();
    for value in &mut kernel {
        *value /= sum;
    }
    let mut horizontal = vec![[0.; 3]; image.len()];
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            for (tap, weight) in kernel.iter().enumerate() {
                let sx = reflected(x as isize + tap as isize - 3, width);
                for channel in 0..3 {
                    horizontal[y * width + x][channel] +=
                        image[y * width + sx][channel] * *weight as f32;
                }
            }
        }
    }
    let mut output = vec![[0.; 3]; image.len()];
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            for (tap, weight) in kernel.iter().enumerate() {
                let sy = reflected(y as isize + tap as isize - 3, height);
                for channel in 0..3 {
                    output[y * width + x][channel] +=
                        horizontal[sy * width + x][channel] * *weight as f32;
                }
            }
        }
    }
    Some(output)
}

pub(super) fn sobel(
    image: &[Ap1],
    width: usize,
    height: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<[f32; 6]>> {
    let mut output = vec![[0.; 6]; image.len()];
    let smooth = [1., 2., 1.];
    let derivative = [-1., 0., 1.];
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            for ky in 0..3 {
                for kx in 0..3 {
                    let sy = reflected(y as isize + ky as isize - 1, height);
                    let sx = reflected(x as isize + kx as isize - 1, width);
                    for channel in 0..3 {
                        output[y * width + x][channel] +=
                            image[sy * width + sx][channel] * derivative[kx] * smooth[ky] / 8.;
                        output[y * width + x][channel + 3] +=
                            image[sy * width + sx][channel] * smooth[kx] * derivative[ky] / 8.;
                    }
                }
            }
        }
    }
    Some(output)
}

fn reflected(mut value: isize, size: usize) -> usize {
    if size == 1 {
        return 0;
    }
    let size = size as isize;
    while value < 0 || value >= size {
        value = if value < 0 {
            -value
        } else {
            2 * size - value - 2
        };
    }
    value as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_distance_transform_preserves_every_reference_distance_bit() {
        assert!(exact_distance_squares((1 << 26) + 1, (1 << 26) + 1));
        assert!(!exact_distance_squares((1 << 26) + 2, (1 << 26) + 1));
        assert!(exact_distance_squares(94_906_266, 1));
        assert!(!exact_distance_squares(94_906_267, 1));
        assert!(!exact_distance_squares(usize::MAX, usize::MAX));
        let distant = [true, false];
        assert_eq!(
            distance_to_unknown(&distant, 94_906_267, 1, &|| false),
            distance_to_unknown_scalar(&distant, 94_906_267, &|| false)
        );
        let check = |known: &[bool], width, height| {
            let exact = distance_to_unknown(known, width, height, &|| false).unwrap();
            let reference = distance_to_unknown_scalar(known, width, &|| false).unwrap();
            assert_eq!(exact, reference, "{width}x{height}");
        };
        for (width, height) in [(1usize, 2048usize), (2048, 1), (257, 7), (63, 41), (64, 64)] {
            for mode in 0..5 {
                let known = (0..width * height)
                    .map(|i| match mode {
                        0 => true,
                        1 => false,
                        2 => i != width * height / 2,
                        3 => (i % width + i / width) % 3 != 0,
                        _ => (i.wrapping_mul(2654435761usize).rotate_left(7) & 15) != 0,
                    })
                    .collect::<Vec<_>>();
                check(&known, width, height);
            }
        }
    }
}
