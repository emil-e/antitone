//! Resident camera-code reads and joint correction/conversion admission.

use crate::Error;
use crate::gpu::evaluation::*;
use crate::photographic::stages::flat_field::GpuFlatField;
use std::hash::{Hash, Hasher};

pub(super) fn validate_camera_range(
    matrix: glam::DMat3,
    field: Option<&GpuFlatField>,
    continuous: bool,
) -> Result<(), Error> {
    let source_gain = if continuous {
        crate::processing::MAX_SOURCE_GAIN
    } else {
        1.
    };
    crate::gpu::parameters::CameraMatrix::new(matrix * source_gain)?;
    if let Some(field) = field {
        field.validate_matrix(matrix, source_gain)?;
    }
    crate::gpu::parameters::CameraMatrix::new(matrix)?;
    Ok(())
}

#[derive(Debug)]
struct ResidentIdentity(std::sync::Weak<()>, bool);
impl PartialEq for ResidentIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0) && self.1 == other.1
    }
}

impl Hash for ResidentIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ptr().hash(state);
        self.1.hash(state);
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
        _budget: crate::gpu::evaluation::Tiling,
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
