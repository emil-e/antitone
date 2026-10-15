// The cached input is corrected camera-to-AP1 source light, without photographic inversion.
// Apply only the display-only source convention and transient inspection EV, before normal ACES.
@compute @workgroup_size(64)
fn inspection_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&working_source) { return; }
    let input = working_source[index];
    if (u32(input.w) & 8u) == 0u { positives[index] = vec4f(0.); return; }
    let gain = Triple(view.gain.x, view.gain.y, view.gain.z);
    var rgb: vec3f;
    for (var c=0u; c<3u; c++) {
        let value = precise_mul(number(input[c]), gain);
        rgb[c] = value.h + (value.l + value.t);
    }
    positives[index] = vec4f(rgb, input.w);
}
