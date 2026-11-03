//! Resident camera samples and their source-coordinate geometry.

use crate::gpu::{flat::GpuFlatField, parameters::CameraMatrix};
use crate::{Error, geometry::SourceRaster, processing::validate_camera_matrix};
use glam::DMat3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[cfg(feature = "editor")]
pub(crate) mod identity;

/// Validated source mapping: a nearest-centre raster or two concatenated original rectangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceGeometry {
    raster: SourceRaster,
    second: Option<SourceRaster>,
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
                    "source geometry exceeds the GPU coordinate domain".into(),
                ));
            }
        }
        if u64::from(size[0]) * u64::from(size[1]) > u32::MAX as u64 {
            return Err(Error::Gpu("source geometry exceeds GPU limits".into()));
        }
        Ok(Self {
            raster: SourceRaster::new(area.into(), size)?,
            second: None,
        })
    }

    /// Integer original coordinates do not need the nearest-centre f32 coordinate restriction.
    pub(crate) fn original_tile(area: [u32; 4], size: [u32; 2]) -> Result<Self, Error> {
        if area[2..] != size || u64::from(size[0]) * u64::from(size[1]) > u32::MAX as u64 {
            return Err(Error::Gpu("invalid original tile geometry".into()));
        }
        Ok(Self {
            raster: SourceRaster::new(area.into(), size)?,
            second: None,
        })
    }

    /// Two real original rectangles, concatenated vertically without resampling.
    #[cfg(all(test, feature = "raw", feature = "export"))]
    pub(crate) fn original_pair(origins: [[u32; 2]; 2], size: [u32; 2]) -> Result<Self, Error> {
        let height = size[1].checked_mul(2).ok_or(Error::NumericalOverflow)?;
        if u64::from(size[0]) * u64::from(height) > u32::MAX as u64 {
            return Err(Error::Gpu("original pair exceeds GPU limits".into()));
        }
        let mut first = Self::new([origins[0][0], origins[0][1], size[0], size[1]], size)?;
        first.second =
            Some(Self::new([origins[1][0], origins[1][1], size[0], size[1]], size)?.raster);
        Ok(first)
    }

    pub(crate) fn raster(&self) -> SourceRaster {
        self.raster
    }

    pub(crate) fn second_raster(&self) -> Option<SourceRaster> {
        self.second
    }

    pub(crate) fn regions(self) -> impl Iterator<Item = crate::geometry::SourceRegion> {
        [Some(self.raster.region()), self.second.map(|r| r.region())]
            .into_iter()
            .flatten()
    }

    pub(crate) fn is_pair(self) -> bool {
        self.second.is_some()
    }

    pub fn pixel(&self, index: u32) -> Result<[u32; 2], Error> {
        let size = self.raster.size();
        if index >= self.pixels() {
            return Err(Error::InvalidParameter("source index outside geometry"));
        }
        let count = size[0] * size[1];
        let (raster, index) = if index >= count {
            (self.second.unwrap(), index - count)
        } else {
            (self.raster, index)
        };
        raster.source_pixel([index % size[0], index / size[0]])
    }

    pub fn region(&self) -> crate::geometry::SourceRegion {
        self.raster.region()
    }

    pub(crate) fn size(&self) -> [u32; 2] {
        let [width, height] = self.raster.size();
        [width, height * if self.is_pair() { 2 } else { 1 }]
    }

    pub(crate) fn pixels(&self) -> u32 {
        let size = self.size();
        size[0] * size[1]
    }
}

pub struct ProcessingSource {
    pub(crate) input: wgpu::Buffer,
    pub(crate) pixels: u32,
    pub(crate) camera_to_ap1: DMat3,
    pub(crate) geometry: Option<SourceGeometry>,
    pub(crate) flat_field: Option<Arc<GpuFlatField>>,
    pub(crate) continuous: bool,
}

impl ProcessingSource {
    #[cfg(feature = "editor")]
    #[cfg(test)]
    pub(crate) fn camera_matrix(&self) -> DMat3 {
        self.camera_to_ap1
    }

    pub(crate) fn with_matrix(self: &Arc<Self>, matrix: DMat3) -> Result<Arc<Self>, Error> {
        if self.camera_to_ap1 == matrix {
            return Ok(self.clone());
        }
        validate_camera_matrix(matrix)?;
        CameraMatrix::new(matrix)?;
        if self.continuous {
            CameraMatrix::new(matrix * crate::processing::MAX_SOURCE_GAIN)?;
        }
        Ok(Arc::new(Self {
            input: self.input.clone(),
            pixels: self.pixels,
            camera_to_ap1: matrix,
            geometry: self.geometry,
            flat_field: self.flat_field.clone(),
            continuous: self.continuous,
        }))
    }

    pub fn new(device: &wgpu::Device, pixels: &[[u16; 4]], matrix: DMat3) -> Result<Self, Error> {
        Self::from_bytes(
            device,
            bytemuck::cast_slice(pixels),
            pixels.len(),
            matrix,
            false,
        )
    }

    /// Continuous camera RGB in native code units, not rounded or packed into RGB16.
    pub fn new_continuous(
        device: &wgpu::Device,
        pixels: &[[f32; 4]],
        matrix: DMat3,
    ) -> Result<Self, Error> {
        Self::validate_continuous(pixels)?;
        Self::from_bytes(
            device,
            bytemuck::cast_slice(pixels),
            pixels.len(),
            matrix,
            true,
        )
    }

    pub(crate) fn validate_continuous(pixels: &[[f32; 4]]) -> Result<(), Error> {
        // A full reduction keeps valid-source checks vectorizable; finite bounds reject NaN/Inf too.
        let valid = pixels.iter().fold(true, |valid, pixel| {
            pixel.iter().fold(valid, |valid, &value| {
                valid & (0. ..=crate::processing::MAX_CAMERA_SOURCE_CODE).contains(&value)
            })
        });
        if !valid {
            return Err(Error::Gpu(
                "continuous camera samples exceed the supported code domain".into(),
            ));
        }
        Ok(())
    }

    fn from_bytes(
        device: &wgpu::Device,
        bytes: &[u8],
        length: usize,
        matrix: DMat3,
        continuous: bool,
    ) -> Result<Self, Error> {
        let count =
            u32::try_from(length).map_err(|_| Error::Gpu("source exceeds GPU limits".into()))?;
        if count == 0
            || bytes.len() as u64 > device.limits().max_storage_buffer_binding_size
            || u64::from(count) * 16 > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::Gpu("source exceeds GPU limits".into()));
        }
        validate_camera_matrix(matrix)?;
        let camera_to_ap1 = matrix;
        CameraMatrix::new(matrix)?;
        if continuous {
            CameraMatrix::new(camera_to_ap1 * crate::processing::MAX_SOURCE_GAIN)?;
        }
        Ok(Self {
            input: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera source samples"),
                contents: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            }),
            pixels: count,
            camera_to_ap1,
            geometry: None,
            flat_field: None,
            continuous,
        })
    }

    pub fn set_geometry(&mut self, geometry: SourceGeometry) -> Result<(), Error> {
        if geometry.pixels() != self.pixels {
            return Err(Error::Gpu(
                "source geometry differs from RGB16 storage".into(),
            ));
        }
        self.geometry = Some(geometry);
        Ok(())
    }

    pub fn set_flat_field(
        &mut self,
        field: Option<Arc<GpuFlatField>>,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        if let Some(field) = &field {
            field.validate_matrix(
                self.camera_to_ap1,
                if self.continuous {
                    crate::processing::MAX_SOURCE_GAIN
                } else {
                    1.
                },
            )?;
        }
        self.set_geometry(geometry)?;
        self.flat_field = field;
        Ok(())
    }

    pub fn write(&self, queue: &wgpu::Queue, pixels: &[[u16; 4]]) -> Result<(), Error> {
        if self.continuous || pixels.len() != self.pixels as usize {
            return Err(Error::Gpu(
                "invalid replacement source tile or format".into(),
            ));
        }
        queue.write_buffer(&self.input, 0, bytemuck::cast_slice(pixels));
        Ok(())
    }

    pub fn write_continuous(&self, queue: &wgpu::Queue, pixels: &[[f32; 4]]) -> Result<(), Error> {
        if !self.continuous || pixels.len() != self.pixels as usize {
            return Err(Error::Gpu(
                "invalid replacement continuous source tile".into(),
            ));
        }
        Self::validate_continuous(pixels)?;
        queue.write_buffer(&self.input, 0, bytemuck::cast_slice(pixels));
        Ok(())
    }

    pub fn byte_len(&self) -> u64 {
        self.input.size()
    }

    pub(crate) fn pixel_count(&self) -> u32 {
        self.pixels
    }

    #[cfg(all(feature = "render", any(feature = "ui", feature = "raw")))]
    pub(crate) fn geometry(&self) -> Option<SourceGeometry> {
        self.geometry
    }
}
