fn field_node(index: u32) -> vec3f {
    let i = 3u * index;
    return vec3f(field_nodes[i], field_nodes[i + 1u], field_nodes[i + 2u]);
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
