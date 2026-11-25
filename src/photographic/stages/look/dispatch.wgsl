@group(0) @binding(0)
var<storage, read> source: array<vec4f>;
@group(0) @binding(1)
var<storage, read_write> destination: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let i = invocation.x + invocation.y * groups.x * 64u;
    if i >= p.dimensions.x {
        return;
    }
    destination[i] = look_pixel(source[i]);
}
