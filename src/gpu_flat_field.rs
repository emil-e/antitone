use super::{CameraMatrix, split_constants};
use crate::{Error, flat_field::FlatField, geometry::SourceRaster};
use bytemuck::Zeroable;
use glam::{DMat3, DVec3, Vec4};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Integer nearest-centre sampling, shared by full-resolution tiles and sparse previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceGeometry {
    raster: SourceRaster,
}
impl SourceGeometry {
    pub fn new(area: [u32; 4], size: [u32; 2]) -> Result<Self, Error> {
        for axis in 0..2 {
            if size[axis] == 0
                || area[axis + 2] == 0
                || u64::from(area[axis]) + u64::from(area[axis + 2]) > 1 << 24
                || (2 * u64::from(size[axis]) - 1) * u64::from(area[axis + 2]) > u32::MAX as u64
                || 2 * u64::from(size[axis]) > u32::MAX as u64
            {
                return Err(Error::Gpu(
                    "source geometry exceeds the precise GPU coordinate domain".into(),
                ));
            }
        }
        if u64::from(size[0]) * u64::from(size[1]) > u32::MAX as u64 {
            return Err(Error::Gpu("source geometry exceeds GPU limits".into()));
        }
        Ok(Self {
            raster: SourceRaster::new(area.into(), size)?,
        })
    }
    pub fn pixel(&self, index: u32) -> Result<[u32; 2], Error> {
        let size = self.raster.size();
        if u64::from(index) >= u64::from(size[0]) * u64::from(size[1]) {
            return Err(Error::InvalidParameter("source index outside geometry"));
        }
        self.raster.source_pixel([index % size[0], index / size[0]])
    }
    pub fn region(&self) -> crate::geometry::SourceRegion {
        self.raster.region()
    }
    #[cfg(feature = "editor")]
    pub(crate) fn size(&self) -> [u32; 2] {
        self.raster.size()
    }
    pub(super) fn pixels(&self) -> u32 {
        let size = self.raster.size();
        size[0] * size[1]
    }
}

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
        let nodes: Vec<[f32; 9]> = field
            .nodes()
            .iter()
            .map(|g| {
                let parts = split_constants(*g, 0.)?;
                Ok(std::array::from_fn(|i| parts[i / 3][i % 3]))
            })
            .collect::<Result<_, Error>>()?;
        if nodes.len() as u64 * 36 > device.limits().max_storage_buffer_binding_size {
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
    pub(super) fn validate_matrix(&self, matrix: DMat3) -> Result<(), Error> {
        let (min, max) = self.bounds;
        if min.min_element() / 65535. < f32::MIN_POSITIVE as f64 {
            return Err(Error::Gpu(
                "corrected camera code can underflow before matrix conversion".into(),
            ));
        }
        CameraMatrix::new(matrix * DMat3::from_diagonal(max))?;
        for (column, gain) in [matrix.x_axis, matrix.y_axis, matrix.z_axis]
            .into_iter()
            .zip(min.to_array())
        {
            if column
                .to_array()
                .iter()
                .any(|v| *v != 0. && v.abs() * gain / 65535. < f32::MIN_POSITIVE as f64)
            {
                return Err(Error::Gpu(
                    "corrected camera-code product can underflow".into(),
                ));
            }
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
}
impl FieldGeometry {
    pub(super) fn uncorrected(source: Option<SourceGeometry>) -> Self {
        let Some(source) = source else {
            return Self::zeroed();
        };
        let area = source.raster.region();
        let size = source.raster.size();
        Self {
            area: [area.x, area.y, area.width, area.height],
            dimensions: [size[0], size[1], 0, 0],
            ..Self::zeroed()
        }
    }

    pub(super) fn new(field: &GpuFlatField, source: SourceGeometry) -> Self {
        let grid = field.field.size();
        let area = source.raster.region();
        let size = source.raster.size();
        Self {
            support: field.field.area(),
            grid: [grid[0] as u32, grid[1] as u32, 1, 0],
            area: [area.x, area.y, area.width, area.height],
            dimensions: [size[0], size[1], 0, 0],
        }
    }
}

pub(super) fn identity_nodes(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("inactive flat-field binding"),
        contents: bytemuck::cast_slice(&[Vec4::ONE]),
        usage: wgpu::BufferUsages::STORAGE,
    })
}
