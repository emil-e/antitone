//! Camera interpretation, working-space conversion and sparse repair operations.

use crate::Error;
use crate::gpu::evaluation::*;
use crate::gpu::{
    flat::{FieldGeometry, GpuFlatField},
    interpretation::InputTransform,
    parameters::ProcessingParameters,
    source::SourceGeometry,
};
use crate::processing::RollSettings;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub enum Interpretation {
    Program(InputTransform, RollSettings),
    Packed(ProcessingParameters),
    Unavailable,
}

impl PartialEq for Interpretation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Program(a, ar), Self::Program(b, br)) => a == b && ar == br,
            (Self::Packed(a), Self::Packed(b)) => bytemuck::bytes_of(a) == bytemuck::bytes_of(b),
            (Self::Unavailable, Self::Unavailable) => true,
            _ => false,
        }
    }
}

impl Interpretation {
    fn parameters(&self) -> Result<ProcessingParameters, Error> {
        match self {
            Self::Program(program, roll) => {
                ProcessingParameters::interpret(program.input, *roll, glam::DMat3::IDENTITY)
            }
            Self::Packed(parameters) => Ok(*parameters),
            Self::Unavailable => Err(Error::InvalidParameter(
                "neutral prefix requires captured input interpretation",
            )),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Mapping {
    Originals,
    Resident(Option<SourceGeometry>),
}

impl Mapping {
    pub fn geometry(self, region: CellRegion) -> Result<Option<SourceGeometry>, Error> {
        match self {
            Self::Originals => {
                let r = region.source()?;
                SourceGeometry::original_tile([r.x, r.y, r.width, r.height], [r.width, r.height])
                    .map(Some)
            }
            Self::Resident(geometry) => Ok(geometry),
        }
    }
}

pub struct CameraInput {
    pub matrix: glam::DMat3,
    pub field: Option<Arc<GpuFlatField>>,
    pub continuous: bool,
    pub mapping: Mapping,
}

impl CameraInput {
    pub(crate) fn resident(
        source: &crate::gpu::source::ProcessingSource,
        program: InputTransform,
        field: Option<Arc<GpuFlatField>>,
    ) -> Self {
        Self {
            matrix: program.matrix,
            field,
            continuous: source.continuous,
            mapping: Mapping::Resident(source.geometry()),
        }
    }
}

pub struct Captured {
    pub interpretation: Interpretation,
    pub matrix: glam::DMat3,
    pub field: Option<Arc<GpuFlatField>>,
    pub mapping: Mapping,
    pub code: Arc<str>,
}

impl Captured {
    pub fn new(
        definition: CameraInput,
        interpretation: Interpretation,
    ) -> Result<Arc<Self>, Error> {
        let CameraInput {
            matrix,
            field,
            continuous,
            mapping,
        } = definition;
        let source_gain = if continuous {
            crate::processing::MAX_SOURCE_GAIN
        } else {
            1.
        };
        crate::gpu::parameters::CameraMatrix::new(matrix * source_gain)?;
        if let Some(field) = &field {
            field.validate_matrix(matrix, source_gain)?;
        }
        crate::gpu::parameters::CameraMatrix::new(matrix)?;
        Ok(Arc::new(Self {
            interpretation,
            matrix,
            field,
            mapping,
            code: crate::gpu::repairs::shader().into(),
        }))
    }

    pub(in crate::gpu) fn bind_field(
        &self,
        function: Function,
        geometry: &FieldGeometry,
    ) -> Function {
        let function = function.with_uniform(5, geometry);
        match &self.field {
            Some(field) => function.with_shared(4, Shared::Buffer(field.nodes.clone())),
            None => function.with_storage(4, bytemuck::bytes_of(&[1f32; 4]).to_vec()),
        }
    }
}

pub struct Camera {
    pub captured: Arc<Captured>,
}

#[derive(Debug, PartialEq)]
struct CameraIdentity {
    field: Option<Arc<crate::flat_field::FlatField>>,
    original_mapping: bool,
    geometry: Option<SourceGeometry>,
}

impl Operation for Camera {
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }

    fn pointwise(&self) -> Option<Function> {
        if !matches!(self.captured.mapping, Mapping::Originals) {
            return None;
        }
        let capture = &self.captured;
        let geometry = match &capture.field {
            Some(field) => FieldGeometry::new(
                field,
                SourceGeometry::new(field.field().area(), [1, 1]).ok()?,
            ),
            None => FieldGeometry::uncorrected(None),
        };
        Some(capture.bind_field(
            Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::shader::functions(),
                    r#"
fn corrected_camera_light(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(camera_light_at(pixel.rgb, vec2u(floor(position))), 0.0);
}
"#
                ),
                "corrected_camera_light",
            ),
            &geometry,
        ))
    }

    fn identity(&self) -> Option<RetainedKey> {
        let (original_mapping, geometry) = match self.captured.mapping {
            Mapping::Originals => (true, None),
            Mapping::Resident(geometry) => (false, geometry),
        };
        Some(RetainedKey::new(CameraIdentity {
            field: self
                .captured
                .field
                .as_ref()
                .map(|field| field.field.clone()),
            original_mapping,
            geometry,
        }))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn lower(
        &self,
        f: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let sample = Sampled::raster(f, input.unwrap());
        self.emit(
            f,
            Some(SampleInput::Callable(sample)),
            output.domain,
            output.region,
        )?
        .materialize(f, output)
    }
}

fn repair_geometry(
    geometry: Option<SourceGeometry>,
) -> Option<(crate::geometry::SourceRegion, SourceGeometry)> {
    let geometry = geometry?;
    let region = geometry.region();
    (geometry.size() == [region.width, region.height]).then_some((region, geometry))
}

pub struct Repairs {
    snapshot: crate::gpu::repairs::Snapshot,
    code: Arc<str>,
    mapping: Mapping,
}

impl Repairs {
    pub fn image(
        input: Arc<ImageNode>,
        snapshot: crate::gpu::repairs::Snapshot,
        code: Arc<str>,
        mapping: Mapping,
    ) -> Arc<ImageNode> {
        if snapshot.0.is_empty() {
            return input;
        }
        ImageNode::new(
            input.domain(),
            Some(input),
            Self {
                snapshot,
                code,
                mapping,
            },
        )
    }
}

impl Operation for Repairs {
    fn identity(&self) -> Option<RetainedKey> {
        #[derive(Debug, PartialEq)]
        struct Repaired(RetainedKey, bool, Option<SourceGeometry>);
        Some(RetainedKey::new(Repaired(
            RetainedKey::new(self.snapshot.content()),
            matches!(self.mapping, Mapping::Originals),
            match self.mapping {
                Mapping::Originals => None,
                Mapping::Resident(geometry) => geometry,
            },
        )))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn in_place(&self) -> bool {
        true
    }

    fn lower(
        &self,
        f: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let Some((region, geometry)) = repair_geometry(self.mapping.geometry(output.region)?)
        else {
            return Ok(());
        };
        let geometry = f.uniform(&FieldGeometry::uncorrected(Some(geometry)));
        let tile = f.uniform(&[
            region.x,
            region.y,
            region.width,
            region.height,
            0u32,
            0,
            0,
            0,
        ]);
        for patch in &self.snapshot.0 {
            let pixels = patch.fragment_pixels(region);
            if pixels.is_empty() {
                continue;
            }
            let count = pixels.len() as u32;
            let data = f.resource(Storage::Data(bytemuck::cast_slice(&pixels).into()));
            f.compute(
                self.code.clone(),
                "repair_neutral",
                vec![
                    (1, Binding::Write(output.resource)),
                    (5, geometry.clone()),
                    (12, Binding::Read(data)),
                    (13, tile.clone()),
                ],
                count,
            );
        }
        Ok(())
    }
}

fn native_parameters(interpretation: &Interpretation) -> Result<ProcessingParameters, Error> {
    let parameters = interpretation.parameters()?;
    if parameters.graded() {
        Ok(parameters)
    } else {
        ProcessingParameters::digital(Default::default(), glam::DMat3::IDENTITY)
    }
}

pub struct Native {
    pub interpretation: Interpretation,
}

impl Native {
    pub fn new(interpretation: Interpretation) -> Result<Self, Error> {
        native_parameters(&interpretation)?;
        Ok(Self { interpretation })
    }
}

#[derive(Debug, PartialEq)]
struct NativeIdentity(Interpretation);

impl Operation for Native {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NativeIdentity(self.interpretation)))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        let parameters =
            native_parameters(&self.interpretation).expect("captured interpretation was admitted");
        Some(
            Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::shader::functions(),
                    include_str!("native.wgsl")
                ),
                "native_response",
            )
            .with_uniform(2, &parameters),
        )
    }
}

pub struct Working {
    interpretation: Interpretation,
    matrix: glam::DMat3,
}

impl Working {
    pub fn new(interpretation: Interpretation, matrix: glam::DMat3) -> Result<Self, Error> {
        interpretation.parameters()?;
        crate::gpu::parameters::CameraMatrix::new(matrix)?;
        Ok(Self {
            interpretation,
            matrix,
        })
    }
}

#[derive(Debug, PartialEq)]
struct WorkingIdentity(Interpretation, glam::DMat3);

impl Operation for Working {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(WorkingIdentity(
            self.interpretation,
            self.matrix,
        )))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        let parameters = self
            .interpretation
            .parameters()
            .expect("captured interpretation was admitted");
        let matrix = crate::gpu::parameters::CameraMatrix::new(self.matrix)
            .expect("captured camera matrix was admitted");
        Some(
            Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::shader::functions(),
                    include_str!("working.wgsl")
                ),
                "native_to_working",
            )
            .with_uniform(2, &parameters)
            .with_uniform(3, &matrix),
        )
    }
}

pub struct ResidentCamera {
    pub domain: Domain,
    pub pixels: wgpu::Buffer,
    pub continuous: bool,
}

impl Operation for ResidentCamera {
    fn input(&self, _: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(None)
    }
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }
    fn lower(
        &self,
        fragment: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        self.emit(fragment, None, output.domain, output.region)?
            .materialize(fragment, output)
    }
}
impl SampleProducer for ResidentCamera {
    fn access(&self) -> Access {
        Access::Callable
    }
    fn emit(
        &self,
        fragment: &mut Fragment,
        _: Option<SampleInput>,
        _: Domain,
        _: CellRegion,
    ) -> Result<Sampled, Error> {
        let b = self.domain.finite()?;
        let bounds = fragment.uniform(&b.packed());
        let (element, value) = if self.continuous {
            ("vec4f", "return resident_codes[index];")
        } else {
            (
                "vec2u",
                "let packed=resident_codes[index]; return vec4f(f32(packed.x & 65535u),f32(packed.x >> 16u),f32(packed.y & 65535u),0.0);",
            )
        };
        Ok(Sampled {
            code: format!(
                r#"
@group(0) @binding(32) var<storage,read> resident_codes: array<{element}>;
@group(0) @binding(33) var<uniform> resident_bounds: vec4u;
fn sample_resident_camera(p:vec2i) -> vec4f {{
    let local=vec2u(p)-resident_bounds.xy;
    let index=local.y*resident_bounds.z+local.x;
    {value}
}}
"#
            )
            .into(),
            symbol: "sample_resident_camera".into(),
            bindings: vec![
                (32, Binding::Shared(Shared::Buffer(self.pixels.clone()))),
                (33, bounds),
            ],
        })
    }
}

impl SampleProducer for Camera {
    fn access(&self) -> Access {
        Access::Callable
    }
    fn emit(
        &self,
        fragment: &mut Fragment,
        input: Option<SampleInput>,
        domain: Domain,
        region: CellRegion,
    ) -> Result<Sampled, Error> {
        let Some(SampleInput::Callable(input)) = input else {
            return Err(Error::InvalidParameter(
                "camera correction requires captured samples",
            ));
        };
        let (function, coordinates) = if let Some(function) = self.pointwise() {
            (function, domain)
        } else {
            let capture = &self.captured;
            let geometry = capture.mapping.geometry(region)?;
            let field = match &capture.field {
                Some(field) => FieldGeometry::new(
                    field,
                    geometry.ok_or(Error::InvalidParameter(
                        "corrected source requires geometry",
                    ))?,
                ),
                None => FieldGeometry::uncorrected(geometry),
            };
            let function = Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::shader::functions(),
                    r#"
fn corrected_resident_camera_light(pixel:vec4f, position:vec2f) -> vec4f {
    var source=vec2u(0u);
    if field.grid.z != 0u {
        let xy=vec2u(floor(position));
        source=source_pixel(xy.y*field.dimensions.x+xy.x);
    }
    return vec4f(camera_light_at(pixel.rgb,source),0.0);
}
"#
                ),
                "corrected_resident_camera_light",
            );
            let function = capture.bind_field(function, &field);
            // Nearest-centre development uses integral proxy indices, not rounded world positions.
            (
                function,
                Domain {
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                    ..domain
                },
            )
        };
        input.map(fragment, coordinates, region, vec![function])
    }
}
