@group(0) @binding(0)
var<storage, read> scans: array<vec2u>;

fn continuous_source() -> bool {
    return camera_matrix.columns[0].w > 0.5;
}

fn source_codes(code: vec2u, index: u32) -> vec3f {
    if continuous_source() {
        let first = scans[index * 2u];
        let second = scans[index * 2u + 1u];
        return vec3f(bitcast<f32>(first.x), bitcast<f32>(first.y), bitcast<f32>(second.x));
    }
    return vec3f(f32(code.x & 65535u), f32(code.x >> 16u), f32(code.y & 65535u));
}

fn convert_source(code: vec2u, index: u32) -> vec3f {
    let codes = source_codes(code, index);
    if field.grid.z == 0u {
        return camera_light_at(codes, vec2u(0u));
    }
    return camera_light_at(codes, source_pixel(field.source, index));
}
