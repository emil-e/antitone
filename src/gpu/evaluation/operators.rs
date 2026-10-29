//! Concrete operation implementations. Geometry and GPU bodies stay out of the planner.

use super::*;
#[cfg(all(test, feature = "editor"))]
use crate::gpu::GpuGrade;
use crate::{geometry::SourceRegion as Rect, rendering::OutputTarget};
#[cfg(all(test, feature = "editor"))]
use wgpu::util::DeviceExt;

pub fn source(source: Arc<dyn Source>) -> Arc<ImageNode> {
    ImageNode::new(source.domain(), None, SourceOp(source))
}

#[cfg(all(test, feature = "editor"))]
impl Source for crate::scan::Scan {
    fn domain(&self) -> Domain {
        Domain {
            bounds: self.area,
            grid: Grid::axis_aligned([0., 0.], [1., 1.]),
            meaning: Meaning::Camera,
        }
    }

    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
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

struct SourceOp(Arc<dyn Source>);

impl Operation for SourceOp {
    fn input(&self, _: Rect) -> Result<Option<Rect>, Error> {
        Ok(None)
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        fragment.work.push(Work::Upload {
            source: self.0.clone(),
            output,
        });
        Ok(())
    }
}

pub fn pointwise(input: Arc<ImageNode>, meaning: Meaning, function: Function) -> Arc<ImageNode> {
    ImageNode::new(
        Domain {
            meaning,
            ..input.domain
        },
        Some(input),
        Map(function),
    )
}

struct Map(Function);

impl Operation for Map {
    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        Some(self.0.clone())
    }

    fn lower(
        &self,
        _: &mut Fragment,
        _: Option<PlannedImage>,
        _: PlannedImage,
    ) -> Result<(), Error> {
        Err(Error::InvalidParameter(
            "pointwise operation requires shader composition",
        ))
    }
}

#[cfg(all(test, feature = "editor"))]
pub fn interpret(
    device: &wgpu::Device,
    input: Arc<ImageNode>,
    matrix: glam::DMat3,
) -> Result<Arc<ImageNode>, Error> {
    if input.domain.meaning != Meaning::Camera || !matrix.is_finite() {
        return Err(Error::InvalidParameter(
            "interpretation requires camera samples",
        ));
    }
    let columns = crate::gpu::CameraMatrix::new(matrix)?.columns;
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
pub fn grade(
    input: Arc<ImageNode>,
    device: &wgpu::Device,
    grade: Arc<GpuGrade>,
) -> Result<Arc<ImageNode>, Error> {
    super::grading::Grading::new(device, &grade, input.domain.grid.coverage).compose(input)
}

pub fn gaussian(input: Arc<ImageNode>, sigma: f64) -> Result<Arc<ImageNode>, Error> {
    if input.domain.meaning != Meaning::Graded || !sigma.is_finite() || sigma <= 0. {
        return Err(Error::InvalidParameter(
            "Gaussian requires graded AP1 and positive sigma",
        ));
    }
    for pitch in input.domain.grid.pitch {
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
    let support = crate::image_mtf::ImageMtfParameters { sigma }
        .support(input.domain.grid.pitch)
        .0;
    Ok(ImageNode::new(
        input.domain,
        Some(input.clone()),
        Gaussian {
            domain: input.domain,
            sigma,
            support,
        },
    ))
}

struct Gaussian {
    domain: Domain,
    sigma: f64,
    support: [u32; 2],
}

impl Gaussian {
    fn parameters(&self, fragment: &mut Fragment, input: PlannedImage) -> Binding {
        let raster = input.domain.grid.raster(input.region);
        fragment.uniform(&crate::gpu::image_mtf::Parameters::new(raster, self.sigma))
    }
}

impl SampleProducer for Gaussian {
    fn access(&self) -> Access {
        Access::Raster
    }

    fn emit(
        &self,
        fragment: &mut Fragment,
        input: SampleInput,
        _domain: Domain,
        _region: Rect,
    ) -> Result<Sampled, Error> {
        let SampleInput::Raster(input) = input else {
            return Err(Error::InvalidParameter("Gaussian requires raster input"));
        };
        let horizontal = fragment.image(input.domain, input.region);
        let parameters = self.parameters(fragment, input);
        fragment.compute(
            crate::gpu::image_mtf::shader(),
            "horizontal",
            vec![
                (1, Binding::Write(horizontal.resource)),
                (10, parameters.clone()),
                (11, Binding::Read(input.resource)),
            ],
            input.cells(),
        );
        let origin = fragment.uniform(&[input.region.x, input.region.y, 0u32, 0]);
        let code = format!(
            "{}\n{}",
            crate::gpu::image_mtf::sample_shader(),
            r#"
@group(0) @binding(12) var<uniform> gaussian_origin: vec4u;

fn sample_gaussian(p: vec2i) -> vec4f {
    let local = p - vec2i(gaussian_origin.xy);
    let index = u32(local.y) * parameters.raster.dimensions.x + u32(local.x);
    return source_mtf(index, vec2i(0, 1), parameters.raster_sigma.y, i32(parameters.radii.y));
}
"#
        );
        Ok(Sampled {
            code: code.into(),
            symbol: "sample_gaussian".into(),
            bindings: vec![
                (10, parameters),
                (11, Binding::Read(horizontal.resource)),
                (12, origin),
            ],
        })
    }
}

impl Operation for Gaussian {
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }

    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(
            crate::processing_support::Support(self.support).expand(region, self.domain.bounds),
        ))
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let horizontal = fragment.image(input.domain, input.region);
        let vertical = fragment.image(input.domain, input.region);
        let parameters = self.parameters(fragment, input);
        let code: Arc<str> = crate::gpu::image_mtf::shader().into();
        for (entry, source, destination) in [
            ("horizontal", input, horizontal),
            ("vertical", horizontal, vertical),
        ] {
            fragment.compute(
                code.clone(),
                entry,
                vec![
                    (1, Binding::Write(destination.resource)),
                    (10, parameters.clone()),
                    (11, Binding::Read(source.resource)),
                ],
                input.cells(),
            );
        }
        let crop = [
            [
                input.region.width,
                input.region.height,
                output.region.width,
                output.region.height,
            ],
            [
                output.region.x - input.region.x,
                output.region.y - input.region.y,
                0,
                0,
            ],
        ];
        let parameters = fragment.uniform(&crop);
        fragment.compute(
            include_str!("../../shaders/raster_crop.wgsl"),
            "main",
            vec![
                (1, Binding::Write(output.resource)),
                (10, parameters),
                (11, Binding::Read(vertical.resource)),
            ],
            output.cells(),
        );
        Ok(())
    }
}

/// Axis-aligned cell-area reconstruction on an explicitly changed grid, before or after grade.
#[cfg(all(test, feature = "editor"))]
pub fn reconstruct(input: Arc<ImageNode>, grid: Domain) -> Result<Arc<ImageNode>, Error> {
    grid.bounds.validate()?;
    if grid.meaning != input.domain.meaning
        || grid.grid.origin.iter().any(|v| !v.is_finite())
        || grid.grid.pitch.iter().any(|v| !v.is_finite() || *v <= 0.)
    {
        return Err(Error::InvalidParameter("invalid reconstruction domain"));
    }
    let base: [f32; 2] = std::array::from_fn(|i| {
        ((grid.grid.origin[i] - input.domain.grid.origin[i]) / input.domain.grid.pitch[i]) as f32
    });
    let scale: [f32; 2] =
        std::array::from_fn(|i| (grid.grid.pitch[i] / input.domain.grid.pitch[i]) as f32);
    // Demand covers both the f64 mapping and its actual f32 shader representation. One-cell
    // roundoff support is sufficient only while multiplication/addition uncertainty is below it.
    for i in 0..2 {
        let end = if i == 0 {
            grid.bounds.x + grid.bounds.width
        } else {
            grid.bounds.y + grid.bounds.height
        };
        let magnitude = f64::from(base[i]).abs() + f64::from(scale[i]) * f64::from(end);
        if !magnitude.is_finite()
            || scale[i] <= 0.
            || 8. * f64::from(f32::EPSILON) * magnitude >= f64::from(scale[i]).min(1.)
        {
            return Err(Error::InvalidParameter(
                "reconstruction exceeds GPU grid precision",
            ));
        }
    }
    Ok(ImageNode::new(
        grid,
        Some(input.clone()),
        Reconstruction {
            input: input.domain,
            output: grid,
            base,
            scale,
        },
    ))
}

#[cfg(all(test, feature = "editor"))]
struct Reconstruction {
    input: Domain,
    output: Domain,
    base: [f32; 2],
    scale: [f32; 2],
}

#[cfg(all(test, feature = "editor"))]
impl Operation for Reconstruction {
    fn sample_consumer(&self) -> Option<&dyn SampleConsumer> {
        Some(self)
    }

    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        let first = [region.x, region.y];
        let end = [region.x + region.width, region.y + region.height];
        let bounds = self.input.bounds;
        let minimum = [bounds.x, bounds.y];
        let maximum = [bounds.x + bounds.width, bounds.y + bounds.height];
        let mut low = [0; 2];
        let mut high = [0; 2];
        for i in 0..2 {
            let exact_low = (self.output.grid.origin[i]
                + f64::from(first[i]) * self.output.grid.pitch[i]
                - self.input.grid.origin[i])
                / self.input.grid.pitch[i];
            let exact_high = (self.output.grid.origin[i]
                + f64::from(end[i]) * self.output.grid.pitch[i]
                - self.input.grid.origin[i])
                / self.input.grid.pitch[i];
            let shader_low =
                f64::from(self.base[i]) + f64::from(first[i]) * f64::from(self.scale[i]);
            let shader_high =
                f64::from(self.base[i]) + f64::from(end[i]) * f64::from(self.scale[i]);
            low[i] = (exact_low.min(shader_low).floor() - 1.)
                .clamp(f64::from(minimum[i]), f64::from(maximum[i] - 1))
                as u32;
            high[i] = (exact_high.max(shader_high).ceil() + 1.)
                .clamp(f64::from(minimum[i] + 1), f64::from(maximum[i]))
                as u32;
        }
        Ok(Some(Rect {
            x: low[0],
            y: low[1],
            width: high[0] - low[0],
            height: high[1] - low[1],
        }))
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let sample = Sampled::raster(fragment, input.unwrap());
        self.emit(fragment, sample, output)
    }
}

#[cfg(all(test, feature = "editor"))]
impl SampleConsumer for Reconstruction {
    fn emit(
        &self,
        fragment: &mut Fragment,
        input: Sampled,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let bounds = self.input.bounds;
        let integers = [
            [0, 0, output.region.width, output.region.height],
            [0, 0, output.region.x, output.region.y],
            [
                bounds.x,
                bounds.y,
                bounds.x + bounds.width,
                bounds.y + bounds.height,
            ],
        ];
        let mut bytes = bytemuck::cast_slice(&integers).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&[
            self.base[0],
            self.base[1],
            self.scale[0],
            self.scale[1],
        ]));
        let uniform = Binding::Uniform(fragment.resource(Storage::Uniform(bytes.into())));
        let code = format!(
            r#"
{}

fn reconstruction_sample(p: vec2i) -> vec4f {{
    return {}(p);
}}

{}
"#,
            input.code,
            input.symbol,
            include_str!("reconstruct.wgsl")
        );
        let mut bindings = input.bindings;
        bindings.extend([(21, Binding::Write(output.resource)), (22, uniform)]);
        fragment.compute(code, "main", bindings, output.cells());
        Ok(())
    }
}

pub fn display(
    input: Arc<ImageNode>,
    renderer: &crate::renderer::GpuRenderer,
    target: OutputTarget,
) -> Result<Arc<ImageNode>, Error> {
    display_with_destination(input, renderer, target, None)
}

pub(crate) fn display_texture(
    input: Arc<ImageNode>,
    renderer: &crate::renderer::GpuRenderer,
    target: OutputTarget,
    texture: &wgpu::Texture,
) -> Result<Arc<ImageNode>, Error> {
    if input.domain.bounds.x != 0
        || input.domain.bounds.y != 0
        || [input.domain.bounds.width, input.domain.bounds.height]
            != [texture.width(), texture.height()]
    {
        return Err(Error::InvalidParameter(
            "display attachment differs from composed image",
        ));
    }
    display_with_destination(input, renderer, target, Some(texture.clone()))
}

fn display_with_destination(
    input: Arc<ImageNode>,
    renderer: &crate::renderer::GpuRenderer,
    target: OutputTarget,
    destination: Option<wgpu::Texture>,
) -> Result<Arc<ImageNode>, Error> {
    if !matches!(input.domain.meaning, Meaning::Graded | Meaning::Neutral) {
        return Err(Error::InvalidParameter(
            "display rendering requires working AP1",
        ));
    }
    let resources = renderer.target(target);
    let operation = Display {
        destination,
        coverage: input.domain.grid.coverage != 0,
        pipeline: resources.pipeline.clone(),
        layout: resources.layout.clone(),
        target,
        shared: vec![
            (1, Shared::Texture(resources.tables[0].clone())),
            (2, Shared::Texture(resources.tables[1].clone())),
            (3, Shared::Sampler(resources.sampler.clone())),
            (5, Shared::Buffer(resources.hues.clone())),
        ],
    };
    Ok(ImageNode::new(
        Domain {
            meaning: Meaning::Display,
            ..input.domain
        },
        Some(input),
        operation,
    ))
}

struct Display {
    destination: Option<wgpu::Texture>,
    coverage: bool,
    target: OutputTarget,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    shared: Vec<(u32, Shared)>,
}

impl SampleProducer for Display {
    fn access(&self) -> Access {
        Access::Callable
    }

    fn emit(
        &self,
        _fragment: &mut Fragment,
        input: SampleInput,
        _domain: Domain,
        _region: Rect,
    ) -> Result<Sampled, Error> {
        let SampleInput::Callable(input) = input else {
            return Err(Error::InvalidParameter(
                "display sampling requires callable input",
            ));
        };
        let code = format!(
            r#"
{}
{}
fn sample_display(p: vec2i) -> vec4f {{
    let pixel = {}(p);
    if pixel.w < 0.0 {{
        return pixel;
    }}

    if {coverage} && (u32(pixel.w) & 8u) == 0u {{
        return vec4f(0.0);
    }}

    if any((bitcast<vec3u>(pixel.rgb) & vec3u(0x7f800000u)) == vec3u(0x7f800000u)) {{
        return vec4f(0.0, 0.0, 0.0, -1.0);
    }}

    return antitone_render(vec4f(pixel.rgb, 1.0));
}}
"#,
            input.code,
            sampling::display_code(self.target)?,
            input.symbol,
            coverage = self.coverage,
        );
        let mut bindings = input.bindings;
        bindings.extend(
            self.shared
                .iter()
                .cloned()
                .map(|(slot, resource)| (slot, Binding::Shared(resource))),
        );
        Ok(Sampled {
            code: code.into(),
            symbol: "sample_display".into(),
            bindings,
        })
    }
}

impl Operation for Display {
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        self.destination.is_none().then_some(self)
    }

    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let texture = if let Some(texture) = &self.destination {
            if output.region != output.domain.bounds {
                return Err(Error::InvalidParameter(
                    "display attachment requires the complete grid",
                ));
            }
            fragment.resources[output.resource.0] = Storage::RenderTarget(texture.clone());
            output.resource
        } else {
            fragment.resource(Storage::Texture(output.size()))
        };
        let mut bindings = vec![
            (0, Binding::Read(input.resource)),
            (
                4,
                fragment.uniform(&[
                    output.region.width,
                    output.region.height,
                    u32::from(self.coverage),
                    0,
                ]),
            ),
        ];
        bindings.extend(
            self.shared
                .iter()
                .cloned()
                .map(|(slot, resource)| (slot, Binding::Shared(resource))),
        );
        fragment.work.push(Work::Render {
            pipeline: self.pipeline.clone(),
            layout: self.layout.clone(),
            bindings,
            target: texture,
        });
        if self.destination.is_some() {
            return Ok(());
        }
        let parameters = fragment.uniform(&[output.region.width, output.region.height, 0u32, 0]);
        fragment.compute(
            include_str!("texture.wgsl"),
            "main",
            vec![
                (0, Binding::Texture(texture)),
                (1, Binding::Write(output.resource)),
                (2, parameters),
                (3, Binding::Read(input.resource)),
            ],
            output.cells(),
        );
        Ok(())
    }
}
