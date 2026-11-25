//! Camera-light normalization and source-coordinate illumination correction.

use super::{GpuFlatField, shader};
use crate::Error;
use crate::gpu::{
    evaluation::*,
    source::{Mapping, SourceGeometry},
};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

pub(crate) fn apply(source: Value, field: Option<Arc<GpuFlatField>>, mapping: Mapping) -> Value {
    Value::from_operation(
        source.domain(),
        Some(source),
        CameraCorrection { field, mapping },
    )
}

pub(crate) struct CameraCorrection {
    pub field: Option<Arc<GpuFlatField>>,
    pub mapping: Mapping,
}

#[derive(Debug, PartialEq)]
pub(crate) struct CameraIdentity {
    pub field: Option<Arc<crate::flat_field::FlatField>>,
    original_mapping: bool,
    geometry: Option<SourceGeometry>,
}

impl Hash for CameraIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Map dimensions locate candidates without reading the gain table.
        self.field
            .as_ref()
            .map(|field| (field.area(), field.size()))
            .hash(state);
        self.original_mapping.hash(state);
        self.geometry.hash(state);
    }
}

impl CameraCorrection {
    fn identity(&self) -> CameraIdentity {
        let (original_mapping, geometry) = match self.mapping {
            Mapping::Originals => (true, None),
            Mapping::Resident(geometry) => (false, geometry),
        };
        CameraIdentity {
            field: self.field.as_ref().map(|field| field.field().clone()),
            original_mapping,
            geometry,
        }
    }
}
impl Operation for CameraCorrection {
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
        _budget: crate::gpu::evaluation::Tiling,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let code = [shader().as_str(), include_str!("apply.wgsl")].join("\n");
        let entry = match self.mapping {
            Mapping::Originals => "original_light",
            Mapping::Resident(_) => "resident_light",
        };
        let function = GpuFlatField::bind(
            self.field.as_deref(),
            self.mapping,
            Function::new(code, entry),
        )?;
        let Function {
            bindings,
            symbol,
            code,
        } = function;
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::new(
                "camera illumination v1",
                self.identity(),
                code,
                symbol,
                value::Access::Pointwise,
            )
            .with_bindings(bindings),
        )
        .map(Some)
    }
}
