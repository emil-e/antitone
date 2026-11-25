fn native_to_working(pixel: vec4f, position: vec2f) -> vec4f {
    let working = camera_positive_to_ap1(array<f32, 3>(pixel.r, pixel.g, pixel.b));
    return finish_pixel(ProcessedPixel(photographic_post(working, 1.0), u32(pixel.a)));
}
