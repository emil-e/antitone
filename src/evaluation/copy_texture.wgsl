@group(0) @binding(0)
var<storage, read> pixels: array<vec4f>;
@group(0) @binding(1)
var output: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2)
var<uniform> source_size: vec4u;

fn source_pixel(p: vec2u) -> vec4f {
    if all(p < source_size.xy) {
        return pixels[p.x + p.y * source_size.x];
    }
    return vec4f(0.0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= textureDimensions(output)) {
        return;
    }
    let pixel = source_pixel(id.xy);
    var value = pixel;
    if pixel.w >= 0.0 {
        value = vec4f(0.0);
        if (u32(pixel.w) & 8u) != 0u {
            value = vec4f(pixel.rgb, 1.0);
        }
    }
    textureStore(output, vec2i(id.xy), value);
}
