//! Concrete operation implementations. Geometry and GPU bodies stay out of the planner.

use super::CellRegion as Rect;
use super::*;
#[cfg(all(test, feature = "editor"))]
use crate::gpu::grading::Grading;
#[cfg(feature = "render")]
use crate::rendering::OutputTarget;
#[cfg(all(test, feature = "editor"))]
use wgpu::util::DeviceExt;

pub fn source(source: Arc<dyn Source>) -> Value {
    Value::from_operation(source.domain(), None, SourceOp(source))
}

#[cfg(all(test, feature = "editor"))]
impl Source for crate::scan::Scan {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(self.area.try_into().unwrap()),
            grid: Grid::axis_aligned([0., 0.], [1., 1.]),
            meaning: Meaning::Camera,
        }
    }

    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        let mut pixels = self.tile_region(
            self.area,
            region.x - self.area.x,
            region.y - self.area.y,
            region.width,
            region.height,
        )?;
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
    fn reads(&self, region: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, region)])
    }
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("image window"))
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
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

    fn reads(&self, _: Rect) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(Vec::new())
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        _: &[crate::gpu::evaluation::value::Value],
        _: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        value::Calculation::from_source(self.0.clone(), output).map(Some)
    }
}

pub fn pointwise(input: Value, meaning: Meaning, function: Function) -> Value {
    value::pointwise(
        input,
        meaning,
        value::Operation::shader_function(None, function),
    )
    .expect("a pointwise function has pointwise access")
}

#[cfg(all(test, feature = "editor"))]
pub fn interpret(device: &wgpu::Device, input: Value, matrix: glam::DMat3) -> Result<Value, Error> {
    if input.domain.meaning != Meaning::Camera || !matrix.is_finite() {
        return Err(Error::InvalidParameter(
            "interpretation requires camera samples",
        ));
    }
    let columns = crate::gpu::parameters::CameraMatrix::new(matrix)?.columns;
    let constants = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("evaluation immutable source interpretation"),
        contents: bytemuck::cast_slice(&columns),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    Ok(pointwise(
        input,
        Meaning::Neutral,
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

pub fn gaussian(input: Value, sigma: f64) -> Result<Value, Error> {
    value::gaussian(input, sigma)
}

#[cfg(feature = "render")]
pub fn display(
    input: Value,
    renderer: &crate::renderer::GpuRenderer,
    target: OutputTarget,
) -> Result<Value, Error> {
    let operation = Display::capture(input.domain, renderer, target)?;
    Ok(Value::from_operation(
        Domain {
            meaning: Meaning::Display,
            ..input.domain
        },
        Some(input),
        operation,
    ))
}

#[cfg(feature = "render")]
struct Display {
    math: Arc<str>,
    coverage: bool,
    target: OutputTarget,
    shared: Vec<(u32, Shared)>,
}

#[cfg(feature = "render")]
impl Display {
    fn capture(
        domain: Domain,
        renderer: &crate::renderer::GpuRenderer,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        if !matches!(
            domain.meaning,
            Meaning::Graded | Meaning::Neutral | Meaning::Inspection
        ) {
            return Err(Error::InvalidParameter(
                "display rendering requires working AP1",
            ));
        }
        let resources = renderer.target(target);
        Ok(Self {
            math: resources.math.clone(),
            coverage: domain.grid.coverage != 0,
            target,
            shared: vec![
                (1, Shared::Texture(resources.tables[0].clone())),
                (2, Shared::Texture(resources.tables[1].clone())),
                (3, Shared::Sampler(resources.sampler.clone())),
                (5, Shared::Buffer(resources.hues.clone())),
            ],
        })
    }
}

#[cfg(feature = "render")]
impl Operation for Display {
    fn key(&self) -> Option<RetainedKey> {
        {
            #[derive(Debug, PartialEq)]
            struct Content(OutputTarget);
            Some(RetainedKey::new(Content(self.target)))
        }
    }

    fn reads(&self, region: Rect) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, region)])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let code = format!(
            r#"
{}
fn pixel(cell:vec2i,position:vec2f,value:vec4f,coverage:vec4u)->vec4f {{
    if value.w<0.0 {{return value;}}
    if coverage.x!=0u && (u32(value.w)&8u)==0u {{return vec4f(0.0);}}
    if any((bitcast<vec3u>(value.rgb)&vec3u(0x7f800000u))==vec3u(0x7f800000u)) {{return vec4f(0.0,0.0,0.0,-1.0);}}
    return vec4f(antitone_render(vec4f(value.rgb,1.0)).rgb,value.w);
}}
"#,
            self.math
        );
        let bindings = self
            .shared
            .iter()
            .map(|(slot, argument)| (*slot, FunctionBinding::Shared(argument.clone())))
            .collect();
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::new(
                "pinned ACES output v1",
                self.target,
                code,
                "pixel",
                value::Access::Pointwise,
            )
            .with_bindings(bindings)
            .with_uniform("vec4u", &[u32::from(self.coverage), 0, 0, 0]),
        )
        .map(Some)
    }
}
