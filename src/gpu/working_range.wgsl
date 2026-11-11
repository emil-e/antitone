// Leave factor-four headroom for sums and interpolation between photographic stages.
// This is arithmetic headroom, not a display-white or scene-exposure limit.
const PHOTOGRAPHIC_LIMIT: f32 = 8.507059173023462e37;

fn photographic_finite(value: f32) -> f32 {
    let bits = bitcast<u32>(value) & 0x7fffffffu;
    if bits > 0x7f800000u {
        return 0.0;
    }
    return clamp(value, -PHOTOGRAPHIC_LIMIT, PHOTOGRAPHIC_LIMIT);
}

fn photographic_exp(value: f32) -> f32 {
    // exp(88) is representable; saturate before exp can overflow, then leave headroom.
    return photographic_finite(exp(min(value, 88.0)));
}

fn photographic_exp2(value: f32) -> f32 {
    return exp2(min(value, 126.0));
}
