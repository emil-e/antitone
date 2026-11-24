@group(0) @binding(18)
var<uniform> curve: vec4f;
fn apply_signed_curve(pixel: vec4f, position: vec2f) -> vec4f {
    if pixel.w < 0.0 || (curve.z != 0.0 && (u32(pixel.w) & 8u) == 0u) {
        return pixel;
    }
    let mapped = pixel.rgb / (vec3f(1.0) + curve.y * abs(pixel.rgb)) * curve.x;
    return vec4f(mapped, pixel.w);
}
