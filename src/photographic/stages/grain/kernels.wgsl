@compute @workgroup_size(64)
fn field(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = raster_index(id, groups);
    if index >= arrayLength(&grain_pixels) {
        return;
    }
    let raster = parameters.raster;
    let pitch = raster_pitch(raster);
    let footprint = max(pitch.x, pitch.y);
    grain_pixels[index] = vec4f(
            grain_field(
                raster_point(raster, index),
                parameters.values.x,
                footprint,
                parameters.seed.x,
            ),
            0.0,
        );
}

@compute @workgroup_size(64)
fn apply(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = raster_index(id, groups);
    if index >= arrayLength(&positives) {
        return;
    }
    positives[index] = apply_grain(positives[index], grain_pixels[index].rgb, parameters);
}
