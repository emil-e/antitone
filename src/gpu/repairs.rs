//! Immutable sparse neutral-AP1 repairs; originals and unmodified view sources remain untouched.
use super::*;
use crate::{dust_repair::EvaluatedRepair, geometry::SourceRegion};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Pixel {
    position: [u32; 4],
    value: [f32; 4],
}

pub(crate) struct Patch {
    pub evaluated: Arc<EvaluatedRepair>,
    buffer: Option<wgpu::Buffer>,
    pub(crate) region: SourceRegion,
    bound: DVec3,
}

impl Patch {
    #[cfg(feature = "editor")]
    pub fn new(device: &wgpu::Device, evaluated: Arc<EvaluatedRepair>) -> Result<Arc<Self>, Error> {
        let mut patch = Self::logical(evaluated)?;
        patch.buffer = patch.buffer_for(device, patch.region)?;
        Ok(Arc::new(patch))
    }

    fn logical(evaluated: Arc<EvaluatedRepair>) -> Result<Self, Error> {
        let support = evaluated.support();
        let [x, y] = support.origin();
        let [width, height] = support.size();
        let mut bound = DVec3::ZERO;
        for (_, value) in evaluated.pixels() {
            if value.iter().any(|value| !value.is_finite()) {
                return Err(Error::Gpu("invalid neutral repair values".into()));
            }
            bound = bound.max(DVec3::from_array(value.map(f64::from)).abs());
        }
        Ok(Self {
            evaluated,
            buffer: None,
            region: SourceRegion {
                x,
                y,
                width,
                height,
            },
            bound,
        })
    }

    fn pixels_in(
        &self,
        region: SourceRegion,
    ) -> impl Iterator<Item = ([u32; 2], crate::dust_repair::Ap1)> + '_ {
        self.evaluated.pixels().filter(move |(point, _)| {
            point[0] >= region.x
                && point[1] >= region.y
                && point[0] < region.x + region.width
                && point[1] < region.y + region.height
        })
    }

    pub(super) fn fragment_pixels(&self, region: SourceRegion) -> Vec<Pixel> {
        if !intersects(self.region, region) {
            return Vec::new();
        }
        let count = self.pixels_in(region).count();
        let mut values = Vec::with_capacity(count);
        let support = self.evaluated.support();
        let [x, y] = support.origin();
        let width = support.size()[0];
        values.extend(self.pixels_in(region).map(|(point, value)| Pixel {
            position: [point[0], point[1], (point[1] - y) * width + point[0] - x, 0],
            value: [value[0], value[1], value[2], 0.],
        }));
        values
    }

    pub(crate) fn buffer_for(
        &self,
        device: &wgpu::Device,
        region: SourceRegion,
    ) -> Result<Option<wgpu::Buffer>, Error> {
        if let Some(buffer) = &self.buffer {
            return Ok(Some(buffer.clone()));
        }
        let evaluated = &self.evaluated;
        let support = evaluated.support();
        let [x, y] = support.origin();
        let [width, _] = support.size();
        let count = self.pixels_in(region).count();
        if count == 0 {
            return Ok(None);
        }
        let length = count as u64 * std::mem::size_of::<Pixel>() as u64;
        if length > device.limits().max_buffer_size
            || length > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::Gpu("repair patch exceeds GPU limits".into()));
        }
        // Fix explicit vector capacity before materializing the upload, rather than growing it.
        let mut pixels = Vec::with_capacity(count);
        pixels.extend(self.pixels_in(region).map(|(point, value)| Pixel {
            position: [point[0], point[1], (point[1] - y) * width + point[0] - x, 0],
            value: [value[0], value[1], value[2], 0.],
        }));
        #[cfg(test)]
        assert_eq!(
            pixels.capacity(),
            count,
            "repair upload bound includes exact host vector capacity"
        );
        let bytes = bytemuck::cast_slice(&pixels);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable sparse neutral AP1 repair"),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        Ok(Some(buffer))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Snapshot(pub Vec<Arc<Patch>>);

impl Snapshot {
    /// Offscreen consumers retain logical sparse content and upload only each demanded original unit.
    pub(crate) fn logical(evaluated: Vec<Arc<EvaluatedRepair>>) -> Result<Self, Error> {
        evaluated
            .into_iter()
            .map(|repair| Patch::logical(repair).map(Arc::new))
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    pub(crate) fn require_area(&self, area: SourceRegion) -> Result<(), Error> {
        for (index, patch) in self.0.iter().enumerate() {
            if !area.contains_region(patch.region)
                || self.0[..index].iter().any(|other| {
                    other
                        .evaluated
                        .support()
                        .intersects(patch.evaluated.support())
                })
            {
                return Err(Error::Gpu(
                    "repair snapshot has overlapping or off-source support".into(),
                ));
            }
        }
        Ok(())
    }

    #[cfg(all(test, feature = "raw", feature = "render"))]
    pub(crate) fn upload_scratch_bytes(&self, region: SourceRegion) -> u64 {
        self.0
            .iter()
            .filter(|patch| intersects(patch.region, region))
            .map(|patch| {
                patch.buffer.as_ref().map_or_else(
                    || {
                        patch.pixels_in(region).count() as u64
                            * std::mem::size_of::<Pixel>() as u64
                            * 2
                    },
                    wgpu::Buffer::size,
                )
            })
            .sum()
    }

    #[cfg(any(feature = "editor", all(test, feature = "raw", feature = "render")))]
    pub(crate) fn byte_len(&self) -> u64 {
        (std::mem::size_of::<Self>() + self.0.capacity() * std::mem::size_of::<Arc<Patch>>()) as u64
            + self
                .0
                .iter()
                .map(|patch| {
                    patch.buffer.as_ref().map_or(0, wgpu::Buffer::size)
                        + patch.evaluated.retained_bytes() as u64
                        + std::mem::size_of::<Patch>() as u64
                })
                .sum::<u64>()
    }

    pub fn bound(&self) -> DVec3 {
        self.0
            .iter()
            .fold(DVec3::ZERO, |bound, patch| bound.max(patch.bound))
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(|(a, b)| {
                Arc::ptr_eq(a, b)
                    || a.evaluated.support() == b.evaluated.support()
                        && a.evaluated
                            .pixels()
                            .zip(b.evaluated.pixels())
                            .all(|(a, b)| {
                                a.0 == b.0 && a.1.map(f32::to_bits) == b.1.map(f32::to_bits)
                            })
            })
    }
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NeutralRepairs")
            .field(&self.0.len())
            .finish()
    }
}

#[cfg(all(test, feature = "render"))]
mod test_support;
#[cfg(all(test, feature = "render"))]
pub(crate) use test_support::Kernels;

fn intersects(a: SourceRegion, b: SourceRegion) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
