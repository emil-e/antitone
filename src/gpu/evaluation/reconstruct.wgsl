@group(0) @binding(21)
var<storage, read_write> output: array<vec4f>;
@group(0) @binding(22)
var<uniform> geometry: Geometry;
@group(0) @binding(23)
var<storage, read_write> metadata: array<f32>;
@group(0) @binding(24)
var<storage, read> means: array<vec4f>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let value = contribution(xy, geometry);
    output[index] = vec4f(value.mean, covered_flags(value.flags, value.mass, geometry));
}

@compute @workgroup_size(64)
fn clear(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&output) {
        return;
    }
    output[index] = vec4f(0.0);
    metadata[index] = 0.0;
}

@compute @workgroup_size(64)
fn accumulate(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let value = contribution(xy, geometry);
    if value.mass <= 0.0 {
        return;
    }
    let local = xy - geometry.origins.xy;
    let destination = local.y * geometry.sizes.y + local.x;
    let previous = output[destination];
    if previous.w == 0.0 {
        output[destination] = vec4f(value.mean, value.mass);
        metadata[destination] = merged_flags(metadata[destination], value.flags);
        return;
    }
    let mass = previous.w + value.mass;
    let weight = value.mass / mass;
    output[destination] = vec4f(weighted_mean(previous.rgb, value.mean, weight), mass);
    metadata[destination] = merged_flags(metadata[destination], value.flags);
}

@compute @workgroup_size(64)
fn finish(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let local = xy - geometry.origins.xy;
    let destination = local.y * geometry.sizes.y + local.x;
    let value = means[index];
    output[destination] = vec4f(value.rgb, covered_flags(metadata[index], value.w, geometry));
}
