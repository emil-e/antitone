@group(0) @binding(18)
var<storage, read> camera_footprint_origins: array<vec2u>;

fn gathered_camera_light(codes: vec4f, position: vec2f) -> vec4f {
    let cell = vec2u(position);
    let original = camera_footprint_origins[cell.y / 8u] + vec2u(cell.x, cell.y % 8u);
    return vec4f(camera_light_at(codes.rgb, original), 0.0);
}
