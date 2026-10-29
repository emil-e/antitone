struct Geometry {
    input: vec4u,
    output: vec4u,
    values: vec4u,
}
@group(0) @binding(0)
var<storage, read> input_pixels: array<vec4f>;
@group(0) @binding(1)
var<storage, read_write> output_pixels: array<vec4f>;
@group(0) @binding(2)
var<uniform> geometry: Geometry;
fn at(point: vec2i) -> vec4f {
    let local = clamp(
            point - vec2i(geometry.input.xy),
            vec2i(0),
            vec2i(geometry.input.zw) - vec2i(1),
        );
    return input_pixels[u32(local.y) * geometry.input.z + u32(local.x)];
}
@compute @workgroup_size(64)
fn convolve(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= geometry.output.z * geometry.output.w {
        return;
    }
    let point =
        vec2i(geometry.output.xy + vec2u(index % geometry.output.z, index / geometry.output.z));
    let center = at(point);
    if center.w < 0.0 || (geometry.values.z != 0u && (u32(center.w) & 8u) == 0u) {
        output_pixels[index] = center;
        return;
    }
    let axis = select(vec2i(1, 0), vec2i(0, 1), geometry.values.y != 0u);
    var mean = vec3f(0.0);
    var total = 0.0;
    for(var i = -1; i <= 1; i++) {
        let sample = at(point + i * i32(geometry.values.x) * axis);
        if sample.w < 0.0 {
            output_pixels[index] = sample;
            return;
        }
        if geometry.values.z != 0u && (u32(sample.w) & 8u) == 0u {
            continue;
        }
        let weight = select(0.25, 0.5, i == 0);
        let next = total + weight;
        let fraction = weight / next;
        mean = mean * (1.0 - fraction) + sample.rgb * fraction;
        total = next;
    }
    output_pixels[index] = vec4f(mean, center.w);
}
