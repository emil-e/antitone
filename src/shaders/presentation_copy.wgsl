// Bridges the float presentation surface to the platform's swapchain format.
// Both sides hold the pipeline's sRGB-encoded values; only the stored precision differs.

@group(0) @binding(0)
var source: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4f,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return VertexOutput(vec4f(positions[index], 0.0, 1.0));
}

@fragment
fn fs(input: VertexOutput) -> @location(0) vec4f {
    // Load by pixel coordinate so the copy keeps the staging orientation unchanged.
    return textureLoad(source, vec2i(input.position.xy), 0);
}
