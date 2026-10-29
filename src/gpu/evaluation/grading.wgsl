struct GradingContext {
    gain: vec4f,
    coverage: vec4u,
}
@group(0) @binding(8)
var<uniform> grading_context: GradingContext;

fn evaluation_grade_covered(pixel: vec4f) -> bool {
    let mask = grading_context.coverage.x;
    return mask == 0u || (u32(pixel.w) & mask) != 0u;
}

fn evaluation_roll(pixel: vec4f, position: vec2f) -> vec4f {
    if !evaluation_grade_covered(pixel) {
        return vec4f(0.0);
    }
    let rgb = photographic_post(array<f32, 3>(pixel.r, pixel.g, pixel.b), grading_context.gain.x);
    return vec4f(rgb[0], rgb[1], rgb[2], pixel.w);
}

fn evaluation_adjustments(pixel: vec4f, position: vec2f) -> vec4f {
    if !evaluation_grade_covered(pixel) {
        return vec4f(0.0);
    }
    let rgb = colour_adjustment_layers(array<f32, 3>(pixel.r, pixel.g, pixel.b), position);
    return vec4f(rgb[0], rgb[1], rgb[2], pixel.w);
}
