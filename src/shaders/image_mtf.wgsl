@group(0) @binding(1)
var<storage, read_write> positives: array<vec4f>;

@compute @workgroup_size(64)
fn horizontal(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = raster_index(id, groups);
    if index >= arrayLength(&working_source) {
        return;
    }
    positives[index] = source_mtf(
            index,
            vec2i(1, 0),
            parameters.raster_sigma.x,
            i32(parameters.radii.x),
        );
}

@compute @workgroup_size(64)
fn vertical(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = raster_index(id, groups);
    if index >= arrayLength(&working_source) {
        return;
    }
    positives[index] = source_mtf(
            index,
            vec2i(0, 1),
            parameters.raster_sigma.y,
            i32(parameters.radii.y),
        );
}
