//! Bounded original evidence and shared-GPU forward responses for the gamma pivot.

use crate::{
    Error,
    color::{AP1_TO_XYZ, Balance, ap1_to_cones, opponent_coordinates},
    coverage::Coverage,
    geometry::SourceRegion,
    gpu::{
        evaluation::{self, CellRegion, Domain, Grid, Region, Tiling, Value},
        flat::GpuFlatField,
    },
    processing::NegativeInput,
    scan::Scan,
};
use glam::{DMat3, DVec2, DVec3};
use std::{collections::BTreeMap, sync::Arc};

const EDGE: u32 = 64;
const LIMIT: u64 = (EDGE * EDGE) as u64;
const BUDGET: Tiling = Tiling {
    tile: [1, EDGE * EDGE],
};

#[cfg(test)]
mod tests;

struct Pixels {
    pixels: Vec<[f32; 4]>,
}

impl evaluation::Source for Pixels {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [1, self.pixels.len() as u32])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        Ok(self.pixels[region.y as usize..(region.y + region.height) as usize].to_vec())
    }
}

/// Corrected, normalized camera light: no gamma, positive conversion or photographic WB baked in.
pub(super) struct Evidence {
    camera: Value,
    weights: Vec<f64>,
}

fn plan(coverage: Coverage, area: SourceRegion) -> Result<Vec<([u32; 2], f64)>, Error> {
    let bounds = coverage
        .bounds(area)
        .map_err(|_| Error::UnusableSample("invalid gamma pivot coverage"))?;
    if u64::from(bounds.width) * u64::from(bounds.height) <= LIMIT {
        return Ok(coverage.pixels(area)?.collect());
    }

    // Stratify the selected quadrilateral itself, not its bounding rectangle. Bilinear
    // Jacobian weights also cover convex non-parallelograms; picker rectangles are affine.
    let [a, b, c, d] = coverage.corners().map(|point| DVec2::from_array(point.0));
    let mut cells = BTreeMap::new();
    for y in 0..EDGE {
        for x in 0..EDGE {
            let u = (f64::from(x) + 0.5) / f64::from(EDGE);
            let v = (f64::from(y) + 0.5) / f64::from(EDGE);
            let point =
                a * ((1. - u) * (1. - v)) + b * (u * (1. - v)) + c * (u * v) + d * ((1. - u) * v);
            let du = (b - a) * (1. - v) + (c - d) * v;
            let dv = (d - a) * (1. - u) + (c - b) * u;
            let weight = du.perp_dot(dv).abs() / LIMIT as f64;
            let cell = point.floor().to_array().map(|value| value as u32);
            if !weight.is_finite() || weight <= 0. || coverage.weight(cell) <= 0. {
                return Err(Error::UnusableSample("invalid gamma pivot coverage"));
            }
            *cells.entry(cell).or_insert(0.) += weight;
        }
    }
    Ok(cells.into_iter().collect())
}

fn read(
    gpu: super::super::measurement::Gpu<'_>,
    image: Value,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<[f32; 4]>, Error> {
    let (context, queue) = gpu;
    let domain = image.domain();
    let request = context.request(
        image,
        Region {
            bounds: domain.finite()?,
            grid: domain.grid,
        },
        BUDGET,
    )?;
    let mut pixels = Vec::new();
    context
        .read(queue, &request, cancelled, |_, block| {
            pixels.extend_from_slice(block);
            Ok(())
        })?
        .ok_or(Error::Cancelled)?;
    Ok(pixels)
}

type Sample = ([f32; 4], f64);

fn camera_samples(
    gpu: super::super::measurement::Gpu<'_>,
    scan: &Scan,
    field: Option<Arc<GpuFlatField>>,
    coverage: Coverage,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Sample>, Error> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let selected = plan(coverage, scan.area)?;
    if selected.is_empty() {
        return Err(Error::UnusableSample("invalid gamma pivot coverage"));
    }
    let positions: Vec<_> = selected.iter().map(|(point, _)| *point).collect();
    let source = scan.source().camera_image()?;
    let codes = positions
        .iter()
        .map(|point| source.pixel_samples(point[0], point[1]))
        .collect::<Result<Vec<_>, _>>()?;
    let camera = crate::photographic::graph::camera_samples(
        evaluation::source(Arc::new(Pixels { pixels: codes })),
        field,
        &positions,
    )?;
    Ok(read(gpu, camera, cancelled)?
        .into_iter()
        .zip(selected.into_iter().map(|(_, weight)| weight))
        .collect())
}

fn usable_camera(pixel: &[f32; 4], floor: f32) -> bool {
    pixel[..3]
        .iter()
        .all(|value| value.is_finite() && *value >= floor)
}

fn usable_positive(pixel: &[f32; 4]) -> bool {
    pixel.iter().all(|value| value.is_finite()) && pixel[3] == 0.
}

fn rgb(pixel: [f32; 4]) -> DVec3 {
    DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64)
}

fn midtones(
    samples: Vec<Sample>,
    positives: Vec<[f32; 4]>,
    camera_to_ap1: DMat3,
) -> Result<Vec<Sample>, Error> {
    let cone_from_camera = ap1_to_cones() * camera_to_ap1;
    let y_from_camera = (AP1_TO_XYZ * camera_to_ap1).row(1);
    let mut eligible = Vec::new();
    for (sample, positive) in samples.into_iter().zip(positives) {
        if !usable_positive(&positive) {
            continue;
        }
        let rgb = rgb(positive);
        let cones = cone_from_camera * rgb;
        let y = y_from_camera.dot(rgb);
        if y.is_finite() && y > 0. && cones.is_finite() && cones.min_element() > 0. {
            eligible.push((y, sample));
        }
    }
    if eligible.len() < 32 {
        return Err(Error::UnusableSample(
            "not enough usable photo samples to lock gamma colour",
        ));
    }

    // Keep the middle luminance half at lock time, including ties at the boundaries.
    eligible.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let quartile = eligible.len() / 4;
    let low = eligible[quartile].0;
    let high = eligible[eligible.len() - quartile - 1].0;
    Ok(eligible
        .into_iter()
        .filter_map(|(y, sample)| (low <= y && y <= high).then_some(sample))
        .collect())
}

impl Evidence {
    fn from_samples(samples: impl IntoIterator<Item = Sample>) -> Self {
        let (pixels, weights) = samples.into_iter().unzip();
        Self {
            camera: evaluation::source(Arc::new(Pixels { pixels })),
            weights,
        }
    }

    pub(super) fn capture_midtones(
        gpu: super::super::measurement::Gpu<'_>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        coverage: Coverage,
        negative: NegativeInput,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, Error> {
        let mut samples = camera_samples(gpu, scan, field, coverage, cancelled)?;
        let floor = scan.effective_floor() as f32;
        samples.retain(|(pixel, _)| usable_camera(pixel, floor));
        if samples.is_empty() {
            return Err(Error::UnusableSample(
                "no usable camera samples for gamma pivot",
            ));
        }
        let pixels = samples.iter().map(|(pixel, _)| *pixel).collect();
        let camera = evaluation::source(Arc::new(Pixels { pixels }));
        let positives = positive_samples(gpu, camera, negative, cancelled)?;
        Ok(Self::from_samples(midtones(
            samples,
            positives,
            scan.camera_to_ap1,
        )?))
    }

    fn mean(
        &self,
        gpu: super::super::measurement::Gpu<'_>,
        negative: NegativeInput,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DVec3, Error> {
        let pixels = positive_samples(gpu, self.camera.clone(), negative, cancelled)?;
        let mut sum = DVec3::ZERO;
        let mut total = 0.;
        for (pixel, weight) in pixels.into_iter().zip(&self.weights) {
            if !usable_positive(&pixel) {
                return Err(Error::UnusableSample(
                    "gamma pivot reaches the source precision floor",
                ));
            }
            sum += rgb(pixel) * *weight;
            total += weight;
        }
        let mean = sum / total;
        if !mean.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(mean)
    }

    pub(super) fn compensate(
        &self,
        gpu: super::super::measurement::Gpu<'_>,
        from: NegativeInput,
        to: NegativeInput,
        matrix: DMat3,
        wb: Balance,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Balance, Error> {
        let before = chromaticity(matrix * self.mean(gpu, from, cancelled)?)?;
        if from == to {
            return Ok(wb);
        }
        let after = chromaticity(matrix * self.mean(gpu, to, cancelled)?)?;
        let correction = before - after;
        Balance {
            warmth: wb.warmth + correction.x,
            tint: wb.tint + correction.y,
        }
        .validate_roll_wb()
        .map_err(|_| Error::UnusableSample("gamma pivot compensation exceeds the Roll WB range"))
    }
}

fn positive_samples(
    gpu: super::super::measurement::Gpu<'_>,
    camera: Value,
    negative: NegativeInput,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<[f32; 4]>, Error> {
    let readout = crate::photographic::graph::camera_response::normalized(camera, negative)?;
    read(gpu, readout, cancelled)
}

fn chromaticity(mean: DVec3) -> Result<DVec2, Error> {
    let cones = ap1_to_cones() * mean;
    if !cones.is_finite() || cones.min_element() <= 0. {
        return Err(Error::UnusableSample(
            "gamma pivot has undefined colour at these settings",
        ));
    }
    Ok(opponent_coordinates(cones.ln()))
}
