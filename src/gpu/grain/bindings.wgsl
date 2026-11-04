struct Grain {
    raster: Raster,
    values: vec4f,
    variance: vec4f,
    seed: vec4u,
}
@group(0) @binding(1)
var<storage, read_write> positives: array<vec4f>;
@group(0) @binding(12)
var<storage, read_write> grain_pixels: array<vec4f>;
@group(0) @binding(10)
var<uniform> parameters: Grain;
