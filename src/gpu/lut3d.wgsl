@group(0) @binding(0)
var<storage, read> source: array<vec4f>;
@group(0) @binding(1)
var<storage, read_write> destination: array<vec4f>;
struct Status {
    flags: atomic<u32>,
    first: atomic<u32>,
}
@group(0) @binding(4)
var<storage, read_write> status: Status;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let i = invocation.x + invocation.y * groups.x * 64u;
    if i >= p.dimensions.x {
        return;
    }
    let result = look_pixel(source[i]);
    if result.w < 0.0 {
        atomicOr(&status.flags, u32(-result.w));
        atomicMin(&status.first, i);
    }
    destination[i] = result;
}
