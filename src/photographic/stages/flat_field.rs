//! Camera-light correction and its immutable, reusable gain map.

use crate::acquisition::camera::{Mapping, SourceGeometry, coordinates};
use crate::evaluation::{Function, Shared};
use crate::photographic::parameters::CameraMatrix;
use crate::{Error, flat_field::FlatField};
use coordinates::SourceCoordinates;
use glam::{DMat3, DVec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub(crate) mod operation;
#[cfg(feature = "editor")]
pub(crate) mod resources;
#[cfg(all(test, target_os = "macos"))]
mod tests;

/// Immutable roll resource; no expanded or corrected full-resolution source is retained.
pub struct GpuFlatField {
    nodes: wgpu::Buffer,
    field: Arc<FlatField>,
    bounds: (DVec3, DVec3),
}

impl GpuFlatField {
    pub fn new(device: &wgpu::Device, field: Arc<FlatField>) -> Result<Self, Error> {
        let a = field.area();
        SourceGeometry::new(a, [1, 1])?;
        let nodes: Vec<[f32; 3]> = field
            .nodes()
            .iter()
            .map(|g| g.as_vec3().to_array())
            .collect();
        if nodes.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::Gpu("flat-field gains exceed the f32 range".into()));
        }
        if nodes.len() as u64 * 12 > device.limits().max_storage_buffer_binding_size {
            return Err(Error::Gpu("flat-field grid exceeds GPU limits".into()));
        }
        Ok(Self {
            nodes: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shared camera RGB flat-field gains"),
                contents: bytemuck::cast_slice(&nodes),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            bounds: field.bounds(),
            field,
        })
    }

    pub fn byte_len(&self) -> u64 {
        self.nodes.size()
    }

    pub fn field(&self) -> &Arc<FlatField> {
        &self.field
    }

    fn bind(field: Option<&Self>, mapping: Mapping, function: Function) -> Result<Function, Error> {
        let geometry = match mapping {
            Mapping::Originals => None,
            Mapping::Resident(geometry) => {
                if field.is_some() && geometry.is_none() {
                    return Err(Error::InvalidParameter(
                        "corrected source requires geometry",
                    ));
                }
                geometry
            }
        };
        Ok(match field {
            Some(field) => function
                .with_uniform(5, &FieldGeometry::new(field, geometry))
                .with_shared(4, Shared::Buffer(field.nodes.clone())),
            None => function
                .with_uniform(5, &FieldGeometry::uncorrected(geometry))
                .with_storage(4, bytemuck::bytes_of(&[1f32; 4]).to_vec()),
        })
    }

    pub(crate) fn validate_matrix(&self, matrix: DMat3, source_gain: f64) -> Result<(), Error> {
        let (_, max) = self.bounds;
        // Camera gains execute before the matrix, so small coefficients cannot rescue overflow.
        if max.max_element() * source_gain > f32::MAX as f64 / 16. {
            return Err(Error::Gpu(
                "corrected camera values exceed the f32 range before matrix conversion".into(),
            ));
        }
        CameraMatrix::new(matrix * DMat3::from_diagonal(max))?;
        if source_gain > 1. {
            CameraMatrix::new(matrix * source_gain * DMat3::from_diagonal(max))?;
        }
        Ok(())
    }
}

pub(crate) fn shader() -> String {
    [
        coordinates::FUNCTIONS,
        include_str!("flat_field/bindings.wgsl"),
        include_str!("flat_field/field.wgsl"),
    ]
    .join("\n")
}

#[cfg(feature = "editor")]
pub(crate) fn gathered(
    source: crate::evaluation::Value,
    field: Option<&GpuFlatField>,
    origins: &[[u32; 2]],
    patch: [u32; 2],
) -> Result<crate::evaluation::Value, Error> {
    let code = [shader().as_str(), include_str!("flat_field/gathered.wgsl")].join("\n");
    let function = Function::new(code, "gathered_camera_light")
        .with_storage(18, bytemuck::cast_slice(origins).to_vec())
        .with_uniform(19, &[patch[0], patch[1], 0u32, 0u32]);
    Ok(crate::evaluation::pointwise(
        source,
        GpuFlatField::bind(field, Mapping::Originals, function)?,
    ))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FieldGeometry {
    support: [u32; 4],
    grid: [u32; 4],
    source: SourceCoordinates,
}

impl FieldGeometry {
    fn uncorrected(source: Option<SourceGeometry>) -> Self {
        Self {
            support: [0; 4],
            grid: [0; 4],
            source: SourceCoordinates::new(source),
        }
    }

    fn new(field: &GpuFlatField, source: Option<SourceGeometry>) -> Self {
        let grid = field.field.size();
        Self {
            support: field.field.area(),
            grid: [grid[0] as u32, grid[1] as u32, 1, 0],
            source: SourceCoordinates::new(source),
        }
    }
}
