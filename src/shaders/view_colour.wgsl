@compute @workgroup_size(64)
fn view_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&working_source) {
        return;
    }
    let input = working_source[index];
    if (u32(input.w) & 8u) == 0u {
        positives[index] = vec4f(0.0);
        return;
    }
    let post = colour_post(array<f32, 3>(input.x, input.y, input.z), view.gain.x);
    let result = colour_locals(colour_global(post), view_point(index));
    positives[index] = vec4f(result[0], result[1], result[2], input.w);
}
