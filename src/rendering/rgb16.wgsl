// Encoded RGB16 plus per-pixel validation status; no file-codec semantics.
@group(0) @binding(0)
var<storage, read> linear: array<vec4f>;
@group(0) @binding(1)
var<storage, read_write> encoded: array<vec2u>;
struct Parameters {
    count: u32,
    peak_bits: u32,
    use_pq: u32,
    padding: u32,
}

@group(0) @binding(2)
var<uniform> parameters: Parameters;

fn transfer(value: f32) -> f32 {
    if parameters.use_pq != 0u {
        // One display-linear unit is 100 nits; PQ's scale is always 10,000 nits.
        let power = pow(value / 100., 2610. / 16384.);
        // Express the ST 2084 ratio as 1 minus a small term to keep it accurate near white.
        let deficit = (672. / 4096. - (21. / 128.) * power) / (1. + (2392. / 128.) * power);
        return pow(1. - deficit, 2523. / 32.);
    }

    if value <= 0.0031308 {
        return 12.92 * value;
    }
    return 1.055 * pow(value, 1.0 / 2.4) - 0.055;
}

fn quantize(value: f32) -> u32 {
    return min(u32(floor(transfer(value) * 65535.0 + 0.5)), 65535u);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= parameters.count {
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
        || any(magnitude.xyz > vec3u(parameters.peak_bits)) {
        encoded[id.x] = vec2u(0u, 2u << 16u);
        return;
    }
    let r = quantize(pixel.r);
    let g = quantize(pixel.g);
    let b = quantize(pixel.b);
    encoded[id.x] = vec2u(r | (g << 16u), b);
}
