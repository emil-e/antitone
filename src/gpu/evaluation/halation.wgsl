struct Geometry {
    dimensions: vec4u,
    offset_radius: vec4u,
    coverage: vec4u,
}
struct Values {
    sigma: vec4f,
    alpha: vec4f,
}
@group(0) @binding(0)
var<storage, read> original: array<vec4f>;
@group(0) @binding(1)
var<storage, read> blurred_x: array<vec4f>;
@group(0) @binding(2)
var<storage, read_write> result: array<vec4f>;
@group(0) @binding(3)
var<uniform> geometry: Geometry;
@group(0) @binding(4)
var<uniform> values: Values;
@group(0) @binding(5)
var<storage, read_write> horizontal_result: array<vec4f>;

fn covered(pixel: vec4f) -> bool {
    let mask = geometry.coverage.x;
    return mask == 0u || (u32(pixel.w) & mask) != 0u;
}
fn weight(offset: i32, sigma: f32) -> f32 {
    let distance = f32(offset) / sigma;
    return exp(-0.5 * distance * distance);
}

@compute @workgroup_size(64)
fn horizontal(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.dimensions.xy;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2i(i32(index % size.x), i32(index / size.x));
    var mean = vec3f(0.0);
    var normalization = 0.0;
    let radius = i32(geometry.offset_radius.z);
    for(var i = -radius; i <= radius; i++) {
        let x = clamp(xy.x + i, 0, i32(size.x) - 1);
        let sample = original[u32(xy.y) * size.x + u32(x)];
        let w = weight(i, values.sigma.x);
        if !covered(sample) || w == 0.0 {
            continue;
        }
        normalization += w;
        // Positive operands make the difference finite and preserve constants exactly.
        mean += (max(sample.rgb, vec3f(0.0)) - mean) * (w / normalization);
    }
    if normalization == 0.0 {
        mean = max(original[index].rgb, vec3f(0.0));
    }
    horizontal_result[index] = vec4f(mean, original[index].w);
}

@compute @workgroup_size(64)
fn combine(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.dimensions.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2i(vec2u(index % size.x, index / size.x) + geometry.offset_radius.xy);
    let input_size = geometry.dimensions.xy;
    let center = u32(xy.y) * input_size.x + u32(xy.x);
    var mean = vec3f(0.0);
    var normalization = 0.0;
    let radius = i32(geometry.offset_radius.w);
    for(var i = -radius; i <= radius; i++) {
        let y = clamp(xy.y + i, 0, i32(input_size.y) - 1);
        let sample = blurred_x[u32(y) * input_size.x + u32(xy.x)];
        let w = weight(i, values.sigma.y);
        if !covered(sample) || w == 0.0 {
            continue;
        }
        normalization += w;
        mean += (sample.rgb - mean) * (w / normalization);
    }
    let source = original[center];
    if normalization == 0.0 {
        mean = max(source.rgb, vec3f(0.0));
    }
    result[index] = vec4f(
            source.rgb + values.alpha.rgb * (mean - max(source.rgb, vec3f(0.0))),
            source.w,
        );
}
