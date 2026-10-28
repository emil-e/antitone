//! Source-anchored grain stage. It consumes only pixels, raster geometry and its own model.
use super::raster::Raster;
use crate::grain::GrainParameters;

pub(crate) fn shader() -> String {
    [
        include_str!("../shaders/raster.wgsl"),
        include_str!("../shaders/grain.wgsl"),
    ]
    .join("\n")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FieldKey {
    pub raster: Raster,
    pub scale: f32,
    pub seed: u32,
}
impl FieldKey {
    pub fn new(raster: Raster, parameters: GrainParameters, seed: u64) -> Self {
        Self {
            raster,
            scale: parameters.grain_size as f32,
            seed: seed as u32 ^ ((seed >> 32) as u32).rotate_left(16),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Parameters {
    raster: Raster,
    values: [f32; 4],
    variance: [f32; 4],
    seed: [u32; 4],
}
pub(crate) fn parameter_data(key: FieldKey, model: GrainParameters) -> Parameters {
    let variance = model.channel_variance(key.raster.footprint());
    Parameters {
        raster: key.raster,
        values: [
            model.grain_size as f32,
            model.amplitude as f32,
            model.colour as f32,
            0.,
        ],
        variance: [
            variance[0] as f32,
            variance[1] as f32,
            variance[2] as f32,
            0.,
        ],
        seed: [key.seed, 0, 0, 0],
    }
}
