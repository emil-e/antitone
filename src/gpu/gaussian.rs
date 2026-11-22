//! Separable Gaussian blur and its input neighbourhood on the captured raster grid.

use super::evaluation::{
    CellRegion, Domain, Operation, Read, RetainedKey, Tiling, Value,
    value::{self, Access, Calculation},
};
use crate::{
    Error,
    gaussian::{RADIUS_SIGMAS, support},
};
use std::sync::Arc;

/// Blur on the input grid, with sigma in source-coordinate units.
pub fn blur(input: Value, sigma: f64) -> Result<Value, Error> {
    if !sigma.is_finite() || sigma <= 0. {
        return Err(Error::InvalidParameter(
            "Gaussian requires finite positive sigma",
        ));
    }
    let domain = input.domain();
    for pitch in domain.grid.pitch {
        let raster_sigma = sigma as f32 / pitch as f32;
        if !raster_sigma.is_finite()
            || raster_sigma <= 0.
            || (RADIUS_SIGMAS * raster_sigma).ceil() as f64 > f64::from(i32::MAX)
        {
            return Err(Error::InvalidParameter(
                "Gaussian parameters exceed GPU representation",
            ));
        }
    }
    Ok(Value::from_operation(
        domain,
        [input],
        Gaussian {
            sigma,
            bounds: domain.bounds,
            support: support(sigma, domain.grid.pitch),
        },
    ))
}

struct Gaussian {
    sigma: f64,
    bounds: Option<CellRegion>,
    support: [u32; 2],
}

#[derive(Debug, PartialEq)]
struct Sigma(f64);

impl std::hash::Hash for Sigma {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        crate::float_hash::f64(self.0, state);
    }
}

#[derive(Debug, PartialEq, Hash)]
struct Pass {
    sigma: Sigma,
    axis: usize,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    bounds: [i32; 4],
    axis: [i32; 4],
    shape: [f32; 4],
}

impl Gaussian {
    fn regions(&self, output: CellRegion) -> Result<(CellRegion, CellRegion), Error> {
        let horizontal = output.expanded([0, self.support[1]], self.bounds)?;
        Ok((
            horizontal,
            horizontal.expanded([self.support[0], 0], self.bounds)?,
        ))
    }

    fn operation(&self, domain: Domain, axis: usize) -> value::Operation {
        let bounds = domain
            .bounds
            .map(|r| [r.x, r.y, r.right(), r.bottom()])
            .unwrap_or([0; 4]);
        let parameters = Parameters {
            bounds,
            axis: [
                i32::from(axis == 0),
                i32::from(axis == 1),
                self.support[axis] as i32,
                domain.grid.coverage as i32,
            ],
            shape: [
                self.sigma as f32 / (domain.grid.pitch[axis] as f32).max(1e-6),
                if domain.bounds.is_some() { 1. } else { 0. },
                0.,
                0.,
            ],
        };
        value::Operation::new(
            "Gaussian image response v1",
            Pass {
                sigma: Sigma(self.sigma),
                axis,
            },
            include_str!("gaussian.wgsl"),
            "pixel",
            Access::Neighborhood,
        )
        .with_uniform("Parameters", &parameters)
    }
}

impl Operation for Gaussian {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("Gaussian group v1", Sigma(self.sigma))))
    }

    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, self.regions(output)?.1)])
    }

    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _budget: Tiling,
    ) -> Result<Option<Calculation>, Error> {
        let inputs = Calculation::references(inputs);
        let (horizontal_region, source_region) = self.regions(output)?;
        let source = inputs[0].request(source_region)?;
        let horizontal = Calculation::new(
            domain,
            horizontal_region,
            vec![source],
            self.operation(domain, 0),
        )?;
        Ok(Some(Calculation::new(
            domain,
            output,
            vec![horizontal],
            self.operation(domain, 1),
        )?))
    }
}
