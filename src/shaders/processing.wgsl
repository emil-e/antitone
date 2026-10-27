fn process_film(scan: vec3f) -> ProcessedPixel {
    let positive = interpret_positive(scan);
    return ProcessedPixel(
            photographic_post(camera_positive_to_ap1(positive.rgb), 1.0),
            positive.flags,
        );
}

fn process_camera_source(scan: vec3f) -> vec4f {
    let working = camera_positive_to_ap1(array<f32, 3>(scan.x, scan.y, scan.z));
    return finish_pixel(ProcessedPixel(photographic_post(working, 1.0), 0u));
}

fn finish_pixel(pixel: ProcessedPixel) -> vec4f {
    return vec4f(pixel.rgb[0], pixel.rgb[1], pixel.rgb[2], f32(pixel.flags));
}

struct Parameters {
    dmin_floor: vec4f,
    gamma_mode: vec4f,
    anchor: vec4f,
    post: array<vec4f, 3>,
    shape: vec4f,
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
        // Graded positive render: interpret the input stage and Roll, then the layer stack.
        let positive = process_film(source);
        positives[index] = finish_pixel(
                ProcessedPixel(adjustment_layer_grade(positive.rgb, index), positive.flags),
            );
    } else {
        // Raw source inspection: no interpretation, no creative grade.
        positives[index] = process_camera_source(source);
    }
}
