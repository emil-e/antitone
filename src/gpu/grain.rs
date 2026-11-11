//! Source-anchored grain stage. It consumes only pixels, raster geometry and its own model.
use super::raster::Raster;
use crate::grain::GrainParameters;

#[cfg(test)]
pub(crate) fn shader() -> String {
    [
        include_str!("raster.wgsl"),
        include_str!("working_range.wgsl"),
        include_str!("grain/parameters.wgsl"),
        include_str!("grain/bindings.wgsl"),
        include_str!("grain/apply.wgsl"),
        include_str!("grain/field.wgsl"),
        include_str!("grain/kernels.wgsl"),
    ]
    .join("\n")
}

pub(crate) fn seed(value: u64) -> u32 {
    value as u32 ^ ((value >> 32) as u32).rotate_left(16)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Parameters {
    raster: Raster,
    values: [f32; 4],
    variance: [f32; 4],
    seed: [u32; 4],
}
pub(crate) fn parameter_data(
    raster: Raster,
    model: GrainParameters,
    noise_seed: u64,
) -> Parameters {
    let variance = model.channel_variance(raster.footprint());
    Parameters {
        raster,
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
        seed: [seed(noise_seed), 0, 0, 0],
    }
}
