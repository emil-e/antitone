struct Parameters {
    dmin_floor: vec4f,
    gamma_mode: vec4f,
    anchor: vec4f,
    post: array<vec4f, 3>,
    dmin_low: vec4f,
    dmin_tail: vec4f,
    gamma_low: vec4f,
    shape: vec4f,
    anchor_low: vec4f,
    post_low: array<vec4f, 3>,
}
@group(0) @binding(0) var<storage, read> scans: array<vec2u>;
@group(0) @binding(1) var<storage, read_write> positives: array<vec4f>;
@group(0) @binding(2) var<uniform> parameters: Parameters;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&scans) { return; }
    let scan = convert_source(scans[index], index);
    if parameters.gamma_mode.w > 0.5 {
        positives[index] = process_film(scan);
    } else {
        positives[index] = process_camera_source(scan);
    }
}
