@group(0) @binding(0)
var<storage, read> patches: array<vec4f>;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    let position = positions[index];
    return VertexOutput(vec4f(position, 0.0, 1.0), position * 0.5 + 0.5);
}

fn extended_srgb(linear: vec3f) -> vec3f {
    let a = abs(linear);
    let encoded = select(
            1.055 * pow(a, vec3f(1.0 / 2.4)) - 0.055,
            12.92 * a,
            a <= vec3f(0.0031308),
        );
    return sign(linear) * encoded;
}

@fragment
fn fs(input: VertexOutput) -> @location(0) vec4f {
    let index = min(
            u32(max(input.uv.x, 0.0) * f32(arrayLength(&patches))),
            arrayLength(&patches) - 1u,
        );
    return vec4f(extended_srgb(patches[index].rgb), 1.0);
}
