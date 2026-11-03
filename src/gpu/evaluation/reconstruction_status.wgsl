fn reconstruction_status(flags: f32, mass: f32, coverage: u32, expected: f32) -> f32 {
    if flags < 0.0 {
        return flags;
    }
    if expected > 0.0 && (mass <= 0.0 || abs(mass - expected) > expected * 0.0001) {
        return -3.0;
    }
    return f32(u32(flags) | select(0u, coverage, mass > 0.0));
}
