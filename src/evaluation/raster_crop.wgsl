struct Crop {
    sizes: vec4u,
    offset: vec4u,
}
@group(0) @binding(1)
var<storage, read_write> output: array<vec4f>;
@group(0) @binding(11)
var<storage, read> input: array<vec4f>;
@group(0) @binding(10)
var<uniform> crop: Crop;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&output) {
        return;
    }
    let x = index % crop.sizes.z + crop.offset.x;
    let y = index / crop.sizes.z + crop.offset.y;
    output[index] = input[y * crop.sizes.x + x];
}
