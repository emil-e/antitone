@group(0) @binding(0)
var image: texture_2d<f32>;
struct Status {
    flags: atomic<u32>,
    first: atomic<u32>,
}
@group(0) @binding(1)
var<storage, read_write> status: Status;
struct Region {
    origin_dx: vec4f,
    dy: vec4f,
}
@group(0) @binding(2)
var<uniform> region: Region;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(image);
    if any(id.xy >= size) {
        return;
    }
    let pixel = textureLoad(image, vec2i(id.xy), 0);
    if pixel.a < 0.
        && overlap(region.origin_dx.xy, vec2f(id.xy), region.origin_dx.zw, region.dy.xy) > 0. {
        atomicOr(&status.flags, u32(-pixel.a));
        atomicMin(&status.first, id.y * size.x + id.x);
    }
}
