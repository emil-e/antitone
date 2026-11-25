fn original_light(cell: vec2i, position: vec2f, value: vec4f) -> vec4f {
    if value.w < 0.0 {
        return value;
    }
    return vec4f(camera_light_at(value.rgb, vec2u(floor(position))), 0.0);
}

fn resident_light(cell: vec2i, position: vec2f, value: vec4f) -> vec4f {
    if value.w < 0.0 {
        return value;
    }
    var source = vec2u(0u);
    if field.grid.z != 0u {
        let xy = vec2u(cell);
        source = source_pixel(field.source, xy.y * field.source.dimensions.x + xy.x);
    }
    return vec4f(camera_light_at(value.rgb, source), 0.0);
}
