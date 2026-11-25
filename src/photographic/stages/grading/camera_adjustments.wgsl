// Camera proxies grade at captured nearest original centres, not footprint centres.
fn camera_adjustments(cell: vec2i, position: vec2f, pixel: vec4f) -> vec4f {
    if pixel.w < 0.0 {
        return pixel;
    }
    let index = u32(cell.x) + u32(cell.y) * field.dimensions.x;
    return evaluation_adjustments(pixel, vec2f(source_pixel(index)) + vec2f(0.5));
}
