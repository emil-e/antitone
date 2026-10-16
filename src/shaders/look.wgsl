// ACEScct/AP1/D60 creative LUT. Finite inputs clip at the declared LUT boundary.
struct Parameters {
    minimum: vec4f,
    maximum: vec4f,
    reciprocal: vec4f,
    dimensions: vec4u,
}
@group(0) @binding(0)
var<storage, read> source: array<vec4f>;
@group(0) @binding(1)
var<storage, read_write> destination: array<vec4f>;
@group(0) @binding(2)
var<storage, read> nodes: array<vec4f>;
@group(0) @binding(3)
var<uniform> p: Parameters;
struct Status {
    flags: atomic<u32>,
    first: atomic<u32>,
}
@group(0) @binding(4)
var<storage, read_write> status: Status;
fn encode(v: f32) -> f32 {
    if v <= 0.0078125 {
        return 10.5402377416545 * v + 0.0729055341958355;
    }
    return (log2(v) + 9.72) / 17.52;
}
fn decode(v: f32) -> f32 {
    if v <= 0.155251141552511 {
        return (v - 0.0729055341958355) / 10.5402377416545;
    }
    return exp2(v * 17.52 - 9.72);
}
fn finite(v: vec3f) -> bool {
    return all(abs(v) <= vec3f(3.4028234663852886e38));
}
fn at(v: vec3u) -> vec3f {
    return nodes[(v.z * p.dimensions.y + v.y) * p.dimensions.y + v.x].rgb;
}
fn reject(i: u32, flags: u32) {
    atomicOr(&status.flags, flags);
    atomicMin(&status.first, i);
    destination[i] = vec4f(0., 0., 0., -f32(flags));
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation: vec3u) {
    let i = invocation.x + invocation.y * p.dimensions.z;
    if i >= p.dimensions.x {
        return;
    }
    let pixel = source[i];
    if p.dimensions.w != 0u && (u32(pixel.a) & 8u) == 0u {
        destination[i] = vec4f(0.);
        return;
    }
    if !finite(pixel.rgb) {
        reject(i, 1u);
        return;
    }
    // Encoding can overflow for finite negative AP1. Clipping also bounds that
    // intermediate infinity; it must not admit a nonfinite source pixel.
    let value = clamp(
            vec3f(encode(pixel.r), encode(pixel.g), encode(pixel.b)),
            p.minimum.xyz,
            p.maximum.xyz,
        );
    let position = clamp((value - p.minimum.xyz) * p.reciprocal.xyz, vec3f(0.), vec3f(1.))
        * f32(p.dimensions.y - 1u);
    var cell = min(vec3u(position), vec3u(p.dimensions.y - 2u));
    let f = position - vec3f(cell);
    var order = array<u32, 3>(0u, 1u, 2u);
    if f[order[0]] < f[order[1]] {
        let a = order[0];
        order[0] = order[1];
        order[1] = a;
    }
    if f[order[1]] < f[order[2]] {
        let a = order[1];
        order[1] = order[2];
        order[2] = a;
    }
    if f[order[0]] < f[order[1]] {
        let a = order[0];
        order[0] = order[1];
        order[1] = a;
    }
    let a = f[order[0]];
    let b = f[order[1]];
    let c = f[order[2]];
    var output = at(cell) * (1. - a);
    cell[order[0]] += 1u;
    output += at(cell) * (a - b);
    cell[order[1]] += 1u;
    output += at(cell) * (b - c);
    cell[order[2]] += 1u;
    output += at(cell) * c;
    let linear = vec3f(decode(output.r), decode(output.g), decode(output.b));
    if !finite(linear) {
        reject(i, 2u);
        return;
    }
    destination[i] = vec4f(linear, pixel.a);
}
