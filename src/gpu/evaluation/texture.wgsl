@group(0) @binding(0)
var image: texture_2d<f32>;
@group(0) @binding(1)
var<storage, read_write> output: array<vec4f>;
@group(0) @binding(2)
var<uniform> size: vec4u;
@group(0) @binding(3)
var<storage, read> working: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= size.x * size.y {
        return;
    }
    let pixel = working[index];
    if pixel.w < 0.0
        || any((bitcast<vec3u>(pixel.rgb) & vec3u(0x7f800000u)) == vec3u(0x7f800000u)) {
        output[index] = vec4f(0.0, 0.0, 0.0, -1.0);
        return;
    }
    output[index] = textureLoad(image, vec2i(i32(index % size.x), i32(index / size.x)), 0);
}
