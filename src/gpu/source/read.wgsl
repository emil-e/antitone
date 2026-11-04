fn source_pixel(index: u32) -> vec2u {
    if field.dimensions.z != 0u {
        let count = field.dimensions.z * field.dimensions.w;
        let tile = index / count;
        let local = index % count;
        let offset = 2u * tile;
        return vec2u(field.packed_origins[offset], field.packed_origins[offset + 1u])
            + vec2u(local % field.dimensions.z, local / field.dimensions.z);
    }
    let xy = vec2u(index % field.dimensions.x, index / field.dimensions.x);
    return field.area.xy + ((2u * xy + vec2u(1u)) * field.area.zw) / (2u * field.dimensions.xy);
}

fn continuous_source() -> bool {
    return camera_matrix.columns[0].w > 0.5;
}

fn source_pixel_count() -> u32 {
    return arrayLength(&scans) / select(1u, 2u, continuous_source());
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
    return camera_light_at(codes, source_pixel(index));
}
