fn local_grade(input: array<f32, 3>, index: u32) -> array<f32, 3> {
    if local_context.count.x == 0u {
        return input;
    }
    let pixel = source_pixel(index);
    return colour_locals(input, vec2f(pixel) + vec2f(0.5));
}
