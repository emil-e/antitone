// The cached input is corrected camera-to-AP1 source light, without photographic inversion.
// Apply viewing WB and exposure in AP1 before normal ACES.
@compute @workgroup_size(64)
fn inspection_main(
    @builtin(global_invocation_id) id: vec3u,
    @builtin(num_workgroups) groups: vec3u,
) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&working_source) {
        return;
    }
    let input = working_source[index];
    if (u32(input.w) & 8u) == 0u {
        positives[index] = vec4f(0.);
        return;
    }
    let transform = mat3x3f(
            view.source_transform[0].xyz,
            view.source_transform[1].xyz,
            view.source_transform[2].xyz,
        );
    positives[index] = vec4f(transform * input.xyz, input.w);
}
