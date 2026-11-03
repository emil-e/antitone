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
