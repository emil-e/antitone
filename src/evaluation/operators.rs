//! Concrete operation implementations. Geometry and GPU bodies stay out of the planner.

use super::CellRegion as Rect;
use super::*;
#[cfg(all(test, feature = "editor"))]
use crate::photographic::stages::grading::Grading;
#[cfg(all(test, feature = "editor"))]
use wgpu::util::DeviceExt;

pub fn source(source: Arc<dyn Source>) -> Value {
    Value::from_operation(source.domain(), None, SourceOp(source))
}

#[cfg(all(test, feature = "editor"))]
impl Source for crate::scan::DevelopedSource {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(self.area().try_into().unwrap()),
            grid: Grid::axis_aligned([0., 0.], [1., 1.]),
        }
    }

    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        self.validate_area(region)?;
        let mut pixels = vec![[0.; 4]; region.width as usize * region.height as usize];
        self.camera_image()?
            .copy_samples(region, &mut pixels, region.width as usize)?;
        // Source adapters expose normalized camera light, not the native code-unit encoding.
        for pixel in &mut pixels {
            for value in &mut pixel[..3] {
                *value /= 65535.;
            }
        }
        Ok(pixels)
    }
}

/// Restrict the result region without resampling or hiding upstream calculations.
pub(crate) fn window(input: Value, bounds: CellRegion) -> Result<Value, Error> {
    if !input.domain.contains_region(bounds) {
        return Err(Error::InvalidParameter(
            "image window exceeds its input domain",
        ));
    }
    if input.domain.bounds == Some(bounds) {
        return Ok(input);
    }
    Ok(Value::from_operation(
        Domain {
            bounds: Some(bounds),
            ..input.domain
        },
        Some(input),
        Window,
    ))
}
struct Window;
impl Operation for Window {
    fn reads(&self, region: CellRegion) -> Result<Vec<crate::evaluation::Read>, Error> {
        Ok(vec![crate::evaluation::Read::new(0, region)])
    }
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("image window"))
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::evaluation::Tiling,
    ) -> Result<Option<crate::evaluation::value::Calculation>, Error> {
        let inputs = crate::evaluation::value::Calculation::references(inputs);
        value::Calculation::window(domain, output, inputs[0].request(output)?).map(Some)
    }
}

struct SourceOp(Arc<dyn Source>);

impl Operation for SourceOp {
    fn source(&self) -> Option<Arc<dyn Source>> {
        Some(self.0.clone())
    }

    fn key(&self) -> Option<RetainedKey> {
        self.0.identity()
    }

    fn reads(&self, _: Rect) -> Result<Vec<crate::evaluation::Read>, Error> {
        Ok(Vec::new())
    }

    fn work_units(&self, demand: Rect, _: Tiling) -> Result<WorkUnits, Error> {
        demand.validate()?;
        match self.0.acquisition_tile() {
            Some(tile) => super::unit::aligned_units(self.0.domain().finite()?, demand, tile),
            None => Ok(Box::new(std::iter::once(demand))),
        }
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        _: &[crate::evaluation::value::Value],
        _: Domain,
        output: CellRegion,
        _budget: crate::evaluation::Tiling,
    ) -> Result<Option<crate::evaluation::value::Calculation>, Error> {
        value::Calculation::from_source(self.0.clone(), output).map(Some)
    }
}

pub fn pointwise(input: Value, function: Function) -> Value {
    value::pointwise(input, value::Operation::shader_function(None, function))
        .expect("a pointwise function has pointwise access")
}

#[cfg(all(test, feature = "editor"))]
pub fn interpret(device: &wgpu::Device, input: Value, matrix: glam::DMat3) -> Result<Value, Error> {
    if !matrix.is_finite() {
        return Err(Error::InvalidParameter(
            "interpretation requires a finite matrix",
        ));
    }
    let columns = crate::photographic::parameters::CameraMatrix::new(matrix)?.columns;
    let constants = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("evaluation immutable source interpretation"),
        contents: bytemuck::cast_slice(&columns),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    Ok(pointwise(
        input,
        Function {
            code: include_str!("matrix.wgsl").into(),
            symbol: "evaluation_interpret",
            bindings: vec![(18, Shared::Buffer(constants).into())],
        },
    ))
}

#[cfg(all(test, feature = "editor"))]
pub fn grade(input: Value, _device: &wgpu::Device, grade: Arc<Grading>) -> Result<Value, Error> {
    grade.for_grid(input.domain.grid.coverage).compose(input)
}
