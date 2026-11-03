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
    var result = positives[index];
    if raster_covered(parameters.raster, result) && parameters.values.y > 0.0 {
        let field = grain_pixels[index].rgb;
        let grain = mix(vec3f(field.y), field, 2.0 * parameters.values.z);
        let amount = parameters.values.y;
        result = vec4f(
                result.rgb * exp(amount * grain - 0.5 * amount * amount * parameters.variance.xyz),
                result.w,
            );
    }
    positives[index] = result;
}
