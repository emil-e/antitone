@group(0) @binding(0)
var<storage, read_write> pixels: array<vec4f>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&pixels) {
        return;
    }
    // Every proxy cell is present; retain the inversion-floor bits alongside coverage.
    pixels[id.x].w = f32(u32(pixels[id.x].w) | 8u);
}
