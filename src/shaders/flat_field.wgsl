struct FieldGeometry {
    support: vec4u,
    grid: vec4u,
    area: vec4u,
    dimensions: vec4u,
    packed_origins: vec4u,
}
@group(0) @binding(4)
var<storage, read> field_nodes: array<f32>;
@group(0) @binding(5)
var<uniform> field: FieldGeometry;

fn field_node(index: u32) -> vec3f {
    let i = 3u * index;
    return vec3f(field_nodes[i], field_nodes[i + 1u], field_nodes[i + 2u]);
}

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

fn field_gains_at(source: vec2u) -> vec3f {
    if any(source < field.support.xy) || any(source >= field.support.xy + field.support.zw) {
        return vec3f(1.0);
    }
    let position = (vec2f(source - field.support.xy) + vec2f(0.5)) / vec2f(field.support.zw)
        * vec2f(field.grid.xy - vec2u(1u));
    let cell = min(vec2u(floor(position)), field.grid.xy - vec2u(2u));
    let fraction = position - vec2f(cell);
    let start = cell.y * field.grid.x + cell.x;
    let a = mix(field_node(start), field_node(start + 1u), fraction.x);
    let b = mix(
            field_node(start + field.grid.x),
            field_node(start + field.grid.x + 1u),
            fraction.x,
        );
    return mix(a, b, fraction.y);
}

fn field_gains(index: u32) -> vec3f {
    return field_gains_at(source_pixel(index));
}

fn camera_light_at(codes: vec3f, source: vec2u) -> vec3f {
    let normalized = codes / 65535.0;
    if field.grid.z == 0u {
        return normalized;
    }
    return normalized * field_gains_at(source);
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
