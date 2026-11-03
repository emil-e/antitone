//! Concrete operation implementations. Geometry and GPU bodies stay out of the planner.

use super::CellRegion as Rect;
use super::*;
#[cfg(all(test, feature = "editor"))]
use crate::gpu::grading::Grading;
#[cfg(feature = "render")]
use crate::rendering::OutputTarget;
#[cfg(all(test, feature = "editor"))]
use wgpu::util::DeviceExt;

pub fn source(source: Arc<dyn Source>) -> Arc<ImageNode> {
    ImageNode::new(source.domain(), None, SourceOp(source))
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
pub(crate) fn window(input: Arc<ImageNode>, bounds: CellRegion) -> Result<Arc<ImageNode>, Error> {
    if !input.domain.contains_region(bounds) {
        return Err(Error::InvalidParameter(
            "image window exceeds its input domain",
        ));
    }
    if input.domain.bounds == Some(bounds) {
        return Ok(input);
    }
    Ok(ImageNode::new(
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
    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("image window"))
    }
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }
}
impl SampleProducer for Window {
    fn access(&self) -> Access {
        Access::Callable
    }
    fn emit(
        &self,
        _: &mut Fragment,
        input: Option<SampleInput>,
        _: Domain,
        _: CellRegion,
    ) -> Result<Sampled, Error> {
        match input {
            Some(SampleInput::Callable(input)) => Ok(input),
            _ => Err(Error::InvalidParameter(
                "image window requires its input calculation",
            )),
        }
    }
}

struct SourceOp(Arc<dyn Source>);

impl Operation for SourceOp {
    fn source(&self) -> Option<Arc<dyn Source>> {
        Some(self.0.clone())
    }

    fn identity(&self) -> Option<RetainedKey> {
        self.0.identity()
    }

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
pub fn grade(
    input: Arc<ImageNode>,
    _device: &wgpu::Device,
    grade: Arc<Grading>,
) -> Result<Arc<ImageNode>, Error> {
    grade.for_grid(input.domain.grid.coverage).compose(input)
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

#[derive(Clone, Debug, PartialEq)]
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
        input: Option<SampleInput>,
        _domain: Domain,
        _region: Rect,
    ) -> Result<Sampled, Error> {
        let Some(SampleInput::Raster(input)) = input else {
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
        let origin = fragment.uniform(&[input.region.x, input.region.y, 0i32, 0]);
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
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.clone()))
    }

    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }

    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region.expanded(self.support, self.domain.bounds)?))
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
                (output.region.x - input.region.x) as u32,
                (output.region.y - input.region.y) as u32,
                0,
                0,
            ],
        ];
        let parameters = fragment.uniform(&crop);
        fragment.compute(
            include_str!("raster_crop.wgsl"),
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

#[cfg(feature = "render")]
pub fn display(
    input: Arc<ImageNode>,
    renderer: &crate::renderer::GpuRenderer,
    target: OutputTarget,
) -> Result<Arc<ImageNode>, Error> {
    let operation = Display::capture(input.domain, renderer, target)?;
    Ok(ImageNode::new(
        Domain {
            meaning: Meaning::Display,
            ..input.domain
        },
        Some(input),
        operation,
    ))
}

#[cfg(feature = "render")]
pub(crate) fn display_texture(
    input: Arc<ImageNode>,
    renderer: &crate::renderer::GpuRenderer,
    target: OutputTarget,
    texture: &wgpu::Texture,
    region: CellRegion,
) -> Result<Arc<ImageNode>, Error> {
    if !input.domain.contains_region(region)
        || [region.width, region.height] != [texture.width(), texture.height()]
    {
        return Err(Error::InvalidParameter(
            "display attachment differs from composed image",
        ));
    }
    let display = Display::capture(input.domain, renderer, target)?;
    Ok(ImageNode::new(
        Domain {
            meaning: Meaning::Display,
            ..input.domain
        },
        Some(input),
        Presentation {
            display,
            texture: texture.clone(),
            region,
        },
    ))
}

#[cfg(feature = "render")]
struct Display {
    coverage: bool,
    target: OutputTarget,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    shared: Vec<(u32, Shared)>,
}

#[cfg(feature = "render")]
impl Display {
    fn capture(
        domain: Domain,
        renderer: &crate::renderer::GpuRenderer,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        if !matches!(domain.meaning, Meaning::Graded | Meaning::Neutral) {
            return Err(Error::InvalidParameter(
                "display rendering requires working AP1",
            ));
        }
        let resources = renderer.target(target);
        Ok(Self {
            coverage: domain.grid.coverage != 0,
            pipeline: resources.pipeline.clone(),
            layout: resources.layout.clone(),
            target,
            shared: vec![
                (1, Shared::Texture(resources.tables[0].clone())),
                (2, Shared::Texture(resources.tables[1].clone())),
                (3, Shared::Sampler(resources.sampler.clone())),
                (5, Shared::Buffer(resources.hues.clone())),
            ],
        })
    }

    fn render(
        &self,
        fragment: &mut Fragment,
        input: PlannedImage,
        output: PlannedImage,
        texture: Resource,
    ) {
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
    }
}

/// The attachment is an encoding-owned sink, not a reusable mathematical value.
#[cfg(feature = "render")]
struct Presentation {
    display: Display,
    texture: wgpu::Texture,
    region: CellRegion,
}

#[cfg(feature = "render")]
impl Operation for Presentation {
    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }
    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        if output.region != self.region {
            return Err(Error::InvalidParameter(
                "display attachment requires the complete grid",
            ));
        }
        fragment.resources[output.resource.0] = Storage::RenderTarget(self.texture.clone());
        self.display
            .render(fragment, input.unwrap(), output, output.resource);
        Ok(())
    }
}

#[cfg(feature = "render")]
impl SampleProducer for Display {
    fn access(&self) -> Access {
        Access::Callable
    }

    fn emit(
        &self,
        _fragment: &mut Fragment,
        input: Option<SampleInput>,
        _domain: Domain,
        _region: Rect,
    ) -> Result<Sampled, Error> {
        let Some(SampleInput::Callable(input)) = input else {
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

    return vec4f(antitone_render(vec4f(pixel.rgb, 1.0)).rgb, pixel.w);
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

#[cfg(feature = "render")]
impl Operation for Display {
    fn identity(&self) -> Option<RetainedKey> {
        #[derive(Debug, PartialEq)]
        struct Content(OutputTarget);
        Some(RetainedKey::new(Content(self.target)))
    }

    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
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
        let texture = fragment.resource(Storage::Texture(output.size()));
        self.render(fragment, input, output, texture);
        let parameters = fragment.uniform(&[
            output.region.width,
            output.region.height,
            u32::from(self.coverage),
            0,
        ]);
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
