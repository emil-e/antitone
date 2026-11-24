fn native_response(pixel: vec4f, position: vec2f) -> vec4f {
    return finish_pixel(interpret_positive(pixel.rgb));
}
