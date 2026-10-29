@group(0) @binding(0)
var<storage, read> pixels: array<vec4f>;
@group(0) @binding(1)
var output: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2)
var<uniform> source_size: vec4u;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(output);
    if any(id.xy >= size) {
        return;
    }
    var value = vec4f(0.);
    if all(id.xy < source_size.xy) {
        value = pixels[id.x + id.y * source_size.x];
    }
    textureStore(output, vec2i(id.xy), value);
}
