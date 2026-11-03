@group(0) @binding(18)
var<storage, read> camera_footprint_origins: array<vec2u>;

@group(0) @binding(19)
var<uniform> camera_patch_size: vec4u;

fn gathered_camera_light(codes: vec4f, position: vec2f) -> vec4f {
    let cell = vec2u(position);
    let height = camera_patch_size.y;
    let original = camera_footprint_origins[cell.y / height] + vec2u(cell.x, cell.y % height);
    return vec4f(camera_light_at(codes.rgb, original), 0.0);
}
