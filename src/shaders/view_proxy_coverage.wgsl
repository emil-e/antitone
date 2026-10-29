@group(0) @binding(0)
var<storage, read_write> pixels: array<vec4f>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&pixels) {
        return;
    }
    // Every proxy cell is present; retain the inversion-floor bits alongside coverage.
    pixels[index].w = f32(u32(pixels[index].w) | 8u);
}
