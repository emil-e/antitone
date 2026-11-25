//! Source-cell mapping for shader consumers of developed camera samples.

use super::SourceGeometry;
use crate::gpu::evaluation::Function;
use bytemuck::Zeroable;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(crate) enum Mapping {
    Originals,
    Resident(Option<SourceGeometry>),
}

pub(crate) const FUNCTIONS: &str = include_str!("coordinates.wgsl");
const BINDING: &str = include_str!("coordinate_binding.wgsl");

pub(crate) fn shader() -> String {
    [FUNCTIONS, BINDING].join("\n")
}

pub(crate) fn bind(function: Function, geometry: SourceGeometry) -> Function {
    function.with_uniform(5, &SourceCoordinates::new(Some(geometry)))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SourceCoordinates {
    area: [u32; 4],
    dimensions: [u32; 4],
    packed_origins: [u32; 4],
}

impl SourceCoordinates {
    pub(crate) fn new(source: Option<SourceGeometry>) -> Self {
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
}
