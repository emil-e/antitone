use super::*;
use crate::gpu::evaluation::Operation as Group;

pub fn source(source: Arc<dyn Source>) -> Value {
    super::super::source(source)
}

pub fn pointwise(input: Value, meaning: Meaning, operation: Operation) -> Result<Value, Error> {
    if operation.access() != Access::Pointwise {
        return Err(Error::InvalidParameter(
            "pointwise node requires same-position input access",
        ));
    }
    Ok(Value::from_operation(
        Domain {
            meaning,
            ..input.domain()
        },
        [input],
        Pointwise(operation),
    ))
}
pub(super) struct Pointwise(pub(super) Operation);
impl Group for Pointwise {
    fn key(&self) -> Option<RetainedKey> {
        self.0.key.clone()
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        Ok(Some(Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            self.0.clone(),
        )?))
    }
}

pub fn gaussian(input: Value, sigma: f64) -> Result<Value, Error> {
    if input.domain().meaning != Meaning::Graded || !sigma.is_finite() || sigma <= 0. {
        return Err(Error::InvalidParameter(
            "Gaussian requires graded AP1 and positive sigma",
        ));
    }
    for pitch in input.domain().grid.pitch {
        let raster_sigma = sigma as f32 / pitch as f32;
        if !raster_sigma.is_finite()
            || raster_sigma <= 0.
            || (4. * raster_sigma).ceil() as f64 > f64::from(i32::MAX)
        {
            return Err(Error::InvalidParameter(
                "Gaussian parameters exceed GPU representation",
            ));
        }
    }
    let bounds = input.domain().bounds;
    let support = crate::image_mtf::ImageMtfParameters { sigma }
        .support(input.domain().grid.pitch)
        .0;
    Ok(Value::from_operation(
        input.domain(),
        [input],
        Gaussian {
            sigma,
            bounds,
            support,
        },
    ))
}
struct Gaussian {
    sigma: f64,
    bounds: Option<CellRegion>,
    support: [u32; 2],
}
#[derive(Debug, PartialEq)]
struct Pass {
    sigma: f64,
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

    fn operation(&self, domain: Domain, axis: usize) -> Operation {
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
        Operation::new(
            "Gaussian image response v1",
            Pass {
                sigma: self.sigma,
                axis,
            },
            include_str!("gaussian.wgsl"),
            "pixel",
            Access::Neighborhood,
        )
        .with_uniform("Parameters", &parameters)
    }
}
impl Group for Gaussian {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("Gaussian group v1", self.sigma)))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, self.regions(output)?.1)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
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
