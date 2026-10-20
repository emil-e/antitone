// Original conversion and post/WB remain compensated before shared FP32 colour grading.
fn colour_round(rgb: array<Triple, 3>) -> array<f32, 3> {
    return array<f32, 3>(colour_value(rgb[0]), colour_value(rgb[1]), colour_value(rgb[2]));
}

fn colour_expansion(rgb: array<f32, 3>) -> array<Triple, 3> {
    return array<Triple, 3>(number(rgb[0]), number(rgb[1]), number(rgb[2]));
}

fn photographic_grade(input: array<Triple, 3>) -> array<Triple, 3> {
    return colour_expansion(colour_global(colour_round(input)));
}

fn local_grade(input: array<Triple, 3>, index: u32) -> array<Triple, 3> {
    if local_context.count.x == 0u {
        return input;
    }
    let pixel = source_pixel(index);
    let point = array<Triple, 2>(
            precise_add(colour_coordinate(pixel.x), number(0.5)),
            precise_add(colour_coordinate(pixel.y), number(0.5)),
        );
    return colour_expansion(colour_locals(colour_round(input), point));
}
