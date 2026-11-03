use crate::gpu::parameters::CameraMatrix;
use crate::gpu::source::SourceGeometry;
use crate::{Error, flat_field::FlatField};
use bytemuck::Zeroable;
use glam::{DMat3, DVec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Immutable roll resource; no expanded or corrected full-resolution source is retained.
pub struct GpuFlatField {
    pub(super) nodes: wgpu::Buffer,
    pub(super) field: Arc<FlatField>,
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

    pub(super) fn validate_matrix(&self, matrix: DMat3, source_gain: f64) -> Result<(), Error> {
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct FieldGeometry {
    support: [u32; 4],
    grid: [u32; 4],
    area: [u32; 4],
    dimensions: [u32; 4],
    packed_origins: [u32; 4],
}

impl FieldGeometry {
    pub(super) fn uncorrected(source: Option<SourceGeometry>) -> Self {
        let Some(source) = source else {
            return Self::zeroed();
        };
        let area = source.raster().region();
        let size = source.size();
        let mut result = Self {
            area: [area.x, area.y, area.width, area.height],
            dimensions: [size[0], size[1], 0, 0],
            ..Self::zeroed()
        };
        if let Some(second) = source.second_raster() {
            let shape = source.raster().size();
            let second = second.region();
            result.dimensions[2..].copy_from_slice(&shape);
            result.packed_origins = [area.x, area.y, second.x, second.y];
        } else if [area.width, area.height] == size {
            result.dimensions[2..].copy_from_slice(&size);
            result.packed_origins = [area.x, area.y, 0, 0];
        }
        result
    }

    pub(super) fn new(field: &GpuFlatField, source: SourceGeometry) -> Self {
        let grid = field.field.size();
        Self {
            support: field.field.area(),
            grid: [grid[0] as u32, grid[1] as u32, 1, 0],
            ..Self::uncorrected(Some(source))
        }
    }
}
