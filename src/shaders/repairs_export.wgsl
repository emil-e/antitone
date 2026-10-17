// Canonical repair inputs are f32 neutral AP1; downstream grading stays compensated.
@compute @workgroup_size(64)
fn repair_export(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&repair_pixels) {
        return;
    }
    let pixel = repair_pixels[id.x];
    let index = repair_index(pixel.position.xy);
    if index == 0xffffffffu {
        return;
    }
    let rgb = repaired_positive(pixel.value.xyz, index);
    let flags = export_pixels[index].high.w;
    export_pixels[index] = ExportPixel(
            vec4f(rgb[0].h, rgb[1].h, rgb[2].h, flags),
            vec4f(rgb[0].l, rgb[1].l, rgb[2].l, 0.),
            vec4f(rgb[0].t, rgb[1].t, rgb[2].t, 0.),
        );
}
