struct ImageMtf {
    raster: Raster,
    raster_sigma: vec4f,
    radii: vec4u,
}
@group(0) @binding(11)
var<storage, read> working_source: array<vec4f>;
@group(0) @binding(10)
var<uniform> parameters: ImageMtf;

fn source_mtf(index: u32, axis: vec2i, sigma: f32, radius: i32) -> vec4f {
    let dimensions = vec2i(parameters.raster.dimensions.xy);
    let xy = vec2i(i32(index) % dimensions.x, i32(index) / dimensions.x);
    var mean = vec3f(0.0);
    var weight_sum = 0.0;
    for(var i = -radius; i <= radius; i++) {
        let p = clamp(xy + i * axis, vec2i(0), dimensions - vec2i(1));
        let sample = working_source[p.x + p.y * dimensions.x];
        if !raster_covered(parameters.raster, sample) {
            continue;
        }
        let weight = exp(-0.5 * f32(i * i) / max(sigma * sigma, 1e-12));
        if weight == 0.0 {
            continue;
        }
        let next_weight_sum = weight_sum + weight;
        let fraction = weight / next_weight_sum;
        // Convex accumulation keeps finite signed/HDR constants finite even for large kernels.
        mean = mean * (1.0 - fraction) + sample.rgb * fraction;
        weight_sum = next_weight_sum;
    }
    if weight_sum <= 0.0 {
        return working_source[index];
    }
    return vec4f(mean, working_source[index].w);
}
