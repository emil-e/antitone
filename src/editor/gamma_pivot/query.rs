//! Bounded original evidence and shared-GPU forward responses for a picked colour.

use crate::{
    Error,
    color::{Balance, ap1_to_cones, opponent_coordinates},
    coverage::Coverage,
    geometry::SourceRegion,
    gpu::{
        evaluation::{self, Budget, CellRegion, Domain, Grid, Meaning, Region, Value},
        flat::GpuFlatField,
    },
    processing::NegativeInput,
    scan::Scan,
};
use glam::{DMat3, DVec2, DVec3};
use std::{collections::BTreeMap, sync::Arc};

const EDGE: u32 = 64;
const LIMIT: u64 = (EDGE * EDGE) as u64;
const BUDGET: Budget = Budget {
    tile: [1, EDGE * EDGE],
    scratch_bytes: 8 << 20,
};

#[cfg(test)]
mod tests;

struct Pixels {
    pixels: Vec<[f32; 4]>,
    meaning: Meaning,
}

impl evaluation::Source for Pixels {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [1, self.pixels.len() as u32])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: self.meaning,
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
    services: &super::super::source::Services,
    image: Value,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<[f32; 4]>, Error> {
    let renderer = services.renderers.get(&services.device, &services.queue)?;
    let domain = image.domain();
    let request = renderer.evaluation().request(
        image,
        Region {
            bounds: domain.finite()?,
            grid: domain.grid,
        },
        BUDGET,
    )?;
    let mut pixels = Vec::new();
    renderer
        .evaluation()
        .read(
            &services.queue,
            services.renderers.submissions(),
            &request,
            cancelled,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )?
        .ok_or(Error::Cancelled)?;
    Ok(pixels)
}

impl Evidence {
    pub(super) fn capture(
        services: &super::super::source::Services,
        scan: &Scan,
        coverage: Coverage,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, Error> {
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
        let field = scan
            .flat_field()
            .cloned()
            .map(|field| GpuFlatField::new(&services.device, field).map(Arc::new))
            .transpose()?;
        let camera = crate::gpu::photograph::PhotographGraph::camera_samples(
            evaluation::source(Arc::new(Pixels {
                pixels: codes,
                meaning: Meaning::CameraCodes,
            })),
            field,
            &positions,
        )?;
        let pixels = read(services, camera, cancelled)?;
        let floor = scan.effective_floor() as f32;
        if pixels.iter().any(|pixel| {
            pixel[..3]
                .iter()
                .any(|value| !value.is_finite() || *value < floor)
        }) {
            return Err(Error::UnusableSample(
                "gamma pivot reaches the source precision floor",
            ));
        }
        Ok(Self {
            camera: evaluation::source(Arc::new(Pixels {
                pixels,
                meaning: Meaning::Camera,
            })),
            weights: selected.into_iter().map(|(_, weight)| weight).collect(),
        })
    }

    fn mean(
        &self,
        services: &super::super::source::Services,
        negative: NegativeInput,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DVec3, Error> {
        let readout =
            crate::gpu::photograph::camera_response::normalized(self.camera.clone(), negative)?;
        let pixels = read(services, readout, cancelled)?;
        let mut sum = DVec3::ZERO;
        let mut total = 0.;
        for (pixel, weight) in pixels.iter().zip(&self.weights) {
            if !pixel.iter().all(|value| value.is_finite()) || pixel[3] != 0. {
                return Err(Error::UnusableSample(
                    "gamma pivot reaches the source precision floor",
                ));
            }
            sum += DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) * *weight;
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
        services: &super::super::source::Services,
        from: NegativeInput,
        to: NegativeInput,
        matrix: DMat3,
        wb: Balance,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Balance, Error> {
        let before = chromaticity(matrix * self.mean(services, from, cancelled)?)?;
        if from == to {
            return Ok(wb);
        }
        let after = chromaticity(matrix * self.mean(services, to, cancelled)?)?;
        let correction = before - after;
        Balance {
            warmth: wb.warmth + correction.x,
            tint: wb.tint + correction.y,
        }
        .validate()
        .map_err(|_| Error::UnusableSample("gamma pivot compensation exceeds the Roll WB range"))
    }
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
