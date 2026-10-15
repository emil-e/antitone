// Precise photographic oracle on the same acquired AP1; reference_locals adapts original masks.
@compute @workgroup_size(64)
fn reference_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&working_source) { return; }
    let input = working_source[index];
    if (u32(input.w) & 8u) == 0u { positives[index] = vec4f(0.0); return; }
    let gain = Triple(view.gain.x, view.gain.y, view.gain.z);
    var rgb = array<Triple, 3>(number(input.x), number(input.y), number(input.z));
    for (var c = 0u; c < 3u; c++) { rgb[c] = precise_mul(rgb[c], gain); }
    rgb = reference_locals(photographic_post(rgb), index);
    positives[index] = finish_pixel(ProcessedPixel(rgb, u32(input.w)));
}
