@group(0) @binding(21)
var<storage, read_write> output: array<vec4f>;
@group(0) @binding(22)
var<uniform> geometry: Geometry;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let value = block_contribution(xy, geometry);
    output[index] = vec4f(value.mean, covered_flags(value.flags, value.mass, geometry));
}
