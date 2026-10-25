struct Parameters {
    dmin_floor: vec4f,
    gamma_mode: vec4f,
    anchor: vec4f,
    post: array<vec4f, 3>,
    shape: vec4f,
    creative: array<vec4f, 3>,
    stage: u32,
}
@group(0) @binding(0)
var<storage, read> scans: array<vec2u>;
@group(0) @binding(1)
var<storage, read_write> positives: array<vec4f>;
@group(0) @binding(2)
var<uniform> parameters: Parameters;

override WORKGROUP_SIZE: u32 = 64u;

@compute @workgroup_size(WORKGROUP_SIZE)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * WORKGROUP_SIZE;
    if index >= source_pixel_count() {
        return;
    }
    let source = convert_source(scans[index], index);
    if parameters.stage != STAGE_SOURCE {
        // Graded positive render: interpret the input stage, then shared grade + locals.
        let global = process_film(source);
        positives[index] =
            finish_pixel(ProcessedPixel(local_grade(global.rgb, index), global.flags));
    } else {
        // Raw source inspection: no interpretation, no creative grade.
        positives[index] = process_camera_source(source);
    }
}
