fn merged_flags(a: f32, b: f32) -> f32 {
    if a < 0.0 || b < 0.0 {
        return min(a, b);
    }
    return f32(u32(a) | u32(b));
}

fn weighted_mean(a: vec3f, b: vec3f, fraction: f32) -> vec3f {
    let difference = b - a;
    if all(abs(difference) <= vec3f(3.402823e38)) {
        if fraction <= 0.5 {
            return a + difference * fraction;
        }
        return b - difference * (1.0 - fraction);
    }
    // Opposite signed HDR endpoints can have an unrepresentable difference.
    return a * (1.0 - fraction) + b * fraction;
}
