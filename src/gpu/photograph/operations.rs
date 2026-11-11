//! Camera interpretation and working-space conversion.

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
    Program(InputTransform),
    Packed(ProcessingParameters),
    Unavailable,
}

impl PartialEq for Interpretation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Program(a), Self::Program(b)) => a == b,
            (Self::Packed(a), Self::Packed(b)) => bytemuck::bytes_of(a) == bytemuck::bytes_of(b),
            (Self::Unavailable, Self::Unavailable) => true,
            _ => false,
        }
    }
}

impl Interpretation {
    fn parameters(&self) -> Result<ProcessingParameters, Error> {
        match self {
            Self::Program(program) => ProcessingParameters::interpret(
                program.input,
                RollSettings::default(),
                glam::DMat3::IDENTITY,
            ),
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

#[derive(Clone)]
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
pub(super) struct CameraIdentity {
    pub field: Option<Arc<crate::flat_field::FlatField>>,
    original_mapping: bool,
    geometry: Option<SourceGeometry>,
}

impl Camera {
    fn identity(&self) -> CameraIdentity {
        let (original_mapping, geometry) = match self.captured.mapping {
            Mapping::Originals => (true, None),
            Mapping::Resident(geometry) => (false, geometry),
        };
        CameraIdentity {
            field: self
                .captured
                .field
                .as_ref()
                .map(|field| field.field.clone()),
            original_mapping,
            geometry,
        }
    }
}
impl Operation for Camera {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.identity()))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let capture = &self.captured;
        let original = matches!(capture.mapping, Mapping::Originals);
        let geometry = if original {
            capture
                .field
                .as_ref()
                .map(|field| SourceGeometry::new(field.field().area(), [1, 1]))
                .transpose()?
        } else {
            capture.mapping.geometry(output)?
        };
        let field = match &capture.field {
            Some(field) => FieldGeometry::new(
                field,
                geometry.ok_or(Error::InvalidParameter(
                    "corrected source requires geometry",
                ))?,
            ),
            None => FieldGeometry::uncorrected(geometry),
        };
        let source = if original {
            "source=vec2u(floor(position));"
        } else {
            "if field.grid.z != 0u { let xy=vec2u(cell);source=source_pixel(xy.y*field.dimensions.x+xy.x); }"
        };
        let code = format!(
            "{}\nfn pixel(cell:vec2i, position:vec2f, value:vec4f)->vec4f {{ if value.w<0.0 {{return value;}} var source=vec2u(0u); {source} return vec4f(camera_light_at(value.rgb,source),0.0); }}",
            crate::gpu::photograph::shader::functions()
        );
        let bindings = capture
            .bind_field(Function::new(code.clone(), "pixel"), &field)
            .bindings;
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::new(
                "camera illumination v1",
                self.identity(),
                code,
                "pixel",
                value::Access::Pointwise,
            )
            .with_bindings(bindings),
        )
        .map(Some)
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
pub(super) struct NativeIdentity(pub Interpretation);

impl Operation for Native {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NativeIdentity(self.interpretation)))
    }

    fn reads(&self, region: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
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
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::pointwise_function(
                "native camera interpretation v1",
                NativeIdentity(self.interpretation),
                self.function(),
            ),
        )
        .map(Some)
    }
}

impl Native {
    pub(super) fn function(&self) -> Function {
        let parameters =
            native_parameters(&self.interpretation).expect("captured interpretation was admitted");
        Function::new(
            format!(
                "{}\n{}",
                crate::gpu::photograph::shader::functions(),
                include_str!("native.wgsl")
            ),
            "native_response",
        )
        .with_uniform(2, &parameters)
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
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(WorkingIdentity(
            self.interpretation,
            self.matrix,
        )))
    }

    fn reads(&self, region: CellRegion) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
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
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::pointwise_function(
                "native to working v1",
                WorkingIdentity(self.interpretation, self.matrix),
                self.function(),
            ),
        )
        .map(Some)
    }
}

impl Working {
    pub(super) fn function(&self) -> Function {
        let parameters = self
            .interpretation
            .parameters()
            .expect("captured interpretation was admitted");
        let matrix = crate::gpu::parameters::CameraMatrix::new(self.matrix)
            .expect("captured camera matrix was admitted");
        Function::new(
            format!(
                "{}\n{}",
                crate::gpu::photograph::shader::functions(),
                include_str!("working.wgsl")
            ),
            "native_to_working",
        )
        .with_uniform(2, &parameters)
        .with_uniform(3, &matrix)
    }
}

#[derive(Debug)]
struct ResidentIdentity(std::sync::Weak<()>, bool);
impl PartialEq for ResidentIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0) && self.1 == other.1
    }
}

pub struct ResidentCamera {
    pub domain: Domain,
    pub identity: std::sync::Weak<()>,
    pub pixels: wgpu::Buffer,
    pub continuous: bool,
}

impl Operation for ResidentCamera {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(ResidentIdentity(
            self.identity.clone(),
            self.continuous,
        )))
    }
    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        _: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let (element, value) = if self.continuous {
            ("vec4f", "return resident_codes[index];")
        } else {
            (
                "vec2u",
                "let packed=resident_codes[index];return vec4f(f32(packed.x&65535u),f32(packed.x>>16u),f32(packed.y&65535u),0.0);",
            )
        };
        let code = format!(
            r#"
@group(0) @binding(0) var<storage,read> resident_codes:array<{element}>;
@group(0) @binding(1) var<uniform> resident_bounds:vec4u;
fn pixel(cell:vec2i,position:vec2f)->vec4f {{
    let local=vec2u(cell)-resident_bounds.xy;
    let index=local.y*resident_bounds.z+local.x;
    {value}
}}"#
        );
        let bounds = self.domain.finite()?.packed();
        let function = Function::new(code.clone(), "pixel")
            .with_shared(0, Shared::Buffer(self.pixels.clone()))
            .with_uniform(1, &bounds);
        value::Calculation::new(
            domain,
            output,
            vec![],
            value::Operation::new(
                "resident camera codes v1",
                ResidentIdentity(self.identity.clone(), self.continuous),
                code,
                "pixel",
                value::Access::Pointwise,
            )
            .with_bindings(function.bindings),
        )
        .map(Some)
    }
}
