@group(0) @binding(0)
var<storage, read> pixels: array<vec4f>;
@group(0) @binding(1)
var<uniform> dimensions: vec4u;
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let points = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(points[index], 0.0, 1.0);
}
@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let xy = min(vec2u(position.xy), dimensions.xy - vec2u(1u));
    return pixels[xy.y * dimensions.x + xy.x];
}
