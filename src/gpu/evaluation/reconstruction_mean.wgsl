fn merged_flags(a: f32, b: f32) -> f32 {
    if a < 0.0 || b < 0.0 {
        return min(a, b);
    }
    return f32(u32(a) | u32(b));
}

fn weighted_mean(a: vec3f, b: vec3f, fraction: f32) -> vec3f {
    let difference = b - a;
    var mean: vec3f;
    if all(abs(difference) <= vec3f(3.402823e38)) {
        if fraction <= 0.5 {
            mean = a + difference * fraction;
        } else {
            mean = b - difference * (1.0 - fraction);
        }
    } else {
        // Opposite signed HDR endpoints can have an unrepresentable difference.
        mean = a * (1.0 - fraction) + b * fraction;
    }

    // Positive area weights cannot produce a channel outside its contributing range.
    let bounded = clamp(mean, min(a, b), max(a, b));
    let nonfinite = (bitcast<vec3u>(mean) & vec3u(0x7f800000u)) == vec3u(0x7f800000u);
    return select(bounded, mean, nonfinite);
}
