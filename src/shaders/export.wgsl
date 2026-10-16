// Preserve compensated AP1 through reconstruction and AP0 conversion.
struct ExportPixel {
    high: vec4f,
    low: vec4f,
    tail: vec4f,
}
@group(0) @binding(10)
var<storage, read_write> export_pixels: array<ExportPixel>;

@compute @workgroup_size(WORKGROUP_SIZE)
fn export_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * WORKGROUP_SIZE;
    if index >= arrayLength(&scans) {
        return;
    }
    let global = process_positive(scans[index], index);
    let rgb = local_grade(global.rgb, index);
    export_pixels[index] = ExportPixel(
            vec4f(rgb[0].h, rgb[1].h, rgb[2].h, f32(global.flags)),
            vec4f(rgb[0].l, rgb[1].l, rgb[2].l, 0.),
            vec4f(rgb[0].t, rgb[1].t, rgb[2].t, 0.),
        );
}
