@group(0) @binding(20)
var<storage, read> camera_adjustment_input: array<vec4f>;
@group(0) @binding(21)
var<storage, read_write> camera_adjustment_output: array<vec4f>;
@group(0) @binding(22)
var<uniform> camera_adjustment_region: vec4u;

// Camera proxies grade at their captured nearest original centres, not footprint centres.
@compute @workgroup_size(64)
fn camera_adjustments(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let i = id.x + id.y * groups.x * 64u;
    if i >= arrayLength(&camera_adjustment_output) {
        return;
    }
    let r = camera_adjustment_region;
    let index = r.x + i % r.z + (r.y + i / r.z) * r.w;
    camera_adjustment_output[i] = evaluation_adjustments(
            camera_adjustment_input[i],
            vec2f(source_pixel(index)) + vec2f(0.5),
        );
}
