// Encoded RGB16 plus per-pixel validation status; no file-codec semantics.
@group(0) @binding(0)
var<storage, read> linear: array<vec4f>;
@group(0) @binding(1)
var<storage, read_write> encoded: array<vec2u>;
@group(0) @binding(2)
var<uniform> dimensions: vec4u;
@group(0) @binding(3)
var<storage, read> thresholds: array<u32>;

fn transfer(value: f32) -> f32 {
    if value <= 0.0031308 {
        return 12.92 * value;
    }
    return 1.055 * pow(value, 1.0 / 2.4) - 0.055;
}

fn bracketed(code: u32, bits: u32) -> bool {
    return (code == 0u || bits >= thresholds[code - 1u])
        && (code == 65535u || bits < thresholds[code]);
}

fn quantize(value: f32, bits: u32) -> u32 {
    // Float transfer is only an index estimate. Exact input boundaries decide the result.
    let guess = min(u32(floor(transfer(value) * 65535.0 + 0.5)), 65535u);
    if bracketed(guess, bits) {
        return guess;
    }
    if guess > 0u && bracketed(guess - 1u, bits) {
        return guess - 1u;
    }
    if guess < 65535u && bracketed(guess + 1u, bits) {
        return guess + 1u;
    }
    // Bounded fallback does not rely on native pow accuracy on another device.
    var low = 0u;
    var high = 65535u;
    for(var step = 0u; step < 16u && low < high; step++) {
        let mid = (low + high) / 2u;
        if bits >= thresholds[mid] {
            low = mid + 1u;
        } else {
            high = mid;
        }
    }
    return low;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= dimensions.x {
        return;
    }
    let pixel = linear[id.x];
    let bits = bitcast<vec4u>(pixel);
    let magnitude = bits & vec4u(0x7fffffffu);
    // Bit checks also reject negative subnormals on devices that flush arithmetic to zero.
    if any((magnitude & vec4u(0x7f800000u)) == vec4u(0x7f800000u))
        || ((bits.w & 0x80000000u) != 0u && magnitude.w != 0u) {
        encoded[id.x] = vec2u(0u, 1u << 16u);
        return;
    }
    if any(((bits.xyz & vec3u(0x80000000u)) != vec3u(0u)) & (magnitude.xyz != vec3u(0u)))
        || any(magnitude.xyz > vec3u(0x3f800000u)) {
        encoded[id.x] = vec2u(0u, 2u << 16u);
        return;
    }
    let r = quantize(pixel.r, magnitude.r);
    let g = quantize(pixel.g, magnitude.g);
    let b = quantize(pixel.b, magnitude.b);
    encoded[id.x] = vec2u(r | (g << 16u), b);
}
