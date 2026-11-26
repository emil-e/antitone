fn native_to_working(pixel: vec4f, position: vec2f) -> vec4f {
    let working = camera_positive_to_ap1(array<f32, 3>(pixel.r, pixel.g, pixel.b));
    let rgb = photographic_post(working, 1.0);
    return vec4f(rgb[0], rgb[1], rgb[2], pixel.w);
}
