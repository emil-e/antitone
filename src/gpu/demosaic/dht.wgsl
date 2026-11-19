// SPDX-License-Identifier: LGPL-2.1-only
// Copyright 2013 Anton Petrusevich. Derived from LibRaw 0.22.3 DHT.
// Antitone modifications: corrected ranges/borders and GPU immutable cleanup.
// See native/dht/README.md and native/dht/LICENSE.LGPL-2.1.
struct Parameters {
    dims: vec4u,
    config: vec4u,
}
struct Bounds {
    lo: vec4u,
    hi: vec4u,
}
struct Ranges {
    lo: vec4f,
    hi: vec4f,
}
@group(0) @binding(0)
var<uniform> args: Parameters;
@group(0) @binding(1)
var<storage, read> original: array<vec2u>;
@group(0) @binding(2)
var<storage, read_write> ma: array<f32>;
@group(0) @binding(3)
var<storage, read_write> mb: array<f32>;
@group(0) @binding(4)
var<storage, read_write> green: array<f32>;
@group(0) @binding(5)
var<storage, read_write> rb: array<vec2f>;
@group(0) @binding(6)
var<storage, read_write> da: array<u32>;
@group(0) @binding(7)
var<storage, read_write> db: array<u32>;
@group(0) @binding(8)
var<storage, read_write> result: array<vec2u>;
@group(0) @binding(9)
var<storage, read_write> partials: array<Bounds>;
@group(0) @binding(10)
var<storage, read_write> ranges: Ranges;
var<workgroup> lows: array<vec4u, 256>;
var<workgroup> highs: array<vec4u, 256>;
const HVSH = 1u;
const HOR = 2u;
const VER = 4u;
const DIASH = 8u;
const LURD = 16u;
const RULD = 32u;
const HOT = 64u;
fn colour(p: vec2i) -> u32 {
    return (args.config.x >> (u32((p.y & 1) * 2 + (p.x & 1)) * 2u)) & 3u;
}
fn index(p: vec2i) -> u32 {
    return u32(p.y) * args.dims.z + u32(p.x);
}
fn inside(p: vec2u) -> bool {
    return all(p >= vec2u(4)) && p.x < args.dims.x + 4u && p.y < args.dims.y + 4u;
}
fn real_sample(p: vec2i) -> u32 {
    let c = colour(p);
    let words = original[u32(p.y) * args.dims.x + u32(p.x)];
    return (words[c / 2u] >> ((c & 1u) * 16u)) & 65535u;
}
fn distance(a: f32, b: f32) -> f32 {
    if a > b {
        return a / b;
    }
    return b / a;
}
fn bound(v: f32, a: f32, b: f32, c: u32) -> f32 {
    var value = v;
    let low = min(a, b) / 1.2;
    let high = max(a, b) * 1.2;
    if value < low {
        let s = low * 0.6;
        let o = low - value;
        value = low - sqrt(s * (o + s)) + s;
    } else if value > high {
        let s = high * 0.4;
        let o = value - high;
        value = high + sqrt(s * (o + s)) - s;
    }
    return clamp(value, ranges.lo[c], ranges.hi[c]);
}
fn m(p: vec2i) -> f32 {
    return mb[index(p)];
}
fn g(p: vec2i) -> f32 {
    return green[index(p)];
}
fn pair(p: vec2i) -> vec2f {
    return rb[u32(p.y) * args.config.y + u32(p.x) / 2u];
}

@compute @workgroup_size(16, 16)
fn prepare(
    @builtin(global_invocation_id) pos: vec3u,
    @builtin(local_invocation_index) tid: u32,
    @builtin(workgroup_id) group: vec3u,
) {
    var low = vec4u(65535);
    var high = vec4u(0);
    if pos.x < args.dims.z && pos.y < args.dims.w {
        let i = pos.y * args.dims.z + pos.x;
        var value = 0.5;
        if inside(pos.xy) {
            let p = vec2i(pos.xy) - vec2i(4);
            let c = colour(p);
            let sample = real_sample(p);
            value = max(f32(sample), 0.5);
            low[c] = sample;
            high[c] = sample;
        }
        ma[i] = value;
        da[i] = 0u;
    }
    lows[tid] = low;
    highs[tid] = high;
    workgroupBarrier();
    for(var step = 128u; step > 0u; step /= 2u) {
        if tid < step {
            lows[tid] = min(lows[tid], lows[tid + step]);
            highs[tid] = max(highs[tid], highs[tid + step]);
        }
        workgroupBarrier();
    }
    if tid == 0u {
        partials[group.y * args.config.z + group.x] = Bounds(lows[0], highs[0]);
    }
}
@compute @workgroup_size(256)
fn reduce_bounds(@builtin(local_invocation_index) tid: u32) {
    var low = vec4u(65535);
    var high = vec4u(0);
    for(var i = tid; i < args.config.w; i += 256u) {
        low = min(low, partials[i].lo);
        high = max(high, partials[i].hi);
    }
    lows[tid] = low;
    highs[tid] = high;
    workgroupBarrier();
    for(var step = 128u; step > 0u; step /= 2u) {
        if tid < step {
            lows[tid] = min(lows[tid], lows[tid + step]);
            highs[tid] = max(highs[tid], highs[tid + step]);
        }
        workgroupBarrier();
    }
    if tid == 0u {
        // Match the current CPU range initialization, including first-CFA minimum bias.
        var minimum = vec4f(0.5);
        let c = colour(vec2i(0));
        minimum[c] = max(f32(lows[0][c]), 0.5);
        ranges.lo = minimum;
        ranges.hi = max(vec4f(highs[0]), vec4f(0.5));
    }
}
fn hot_sample(i: u32, from_a: bool) -> f32 {
    if from_a {
        return ma[i];
    }
    return mb[i];
}
fn hot_value(i: u32, from_a: bool) -> vec2f {
    let pitch = args.dims.z;
    let two = 2u * pitch;
    let c = hot_sample(i, from_a);
    let right = hot_sample(i + 2u, from_a);
    var extreme = false;
    if c > right {
        extreme =
            c > hot_sample(i - 2u, from_a) && c > hot_sample(i - two, from_a)
            && c > hot_sample(i + two, from_a) && c > hot_sample(i + 1u, from_a)
            && c > hot_sample(i - 1u, from_a) && c > hot_sample(i - pitch, from_a)
            && c > hot_sample(i + pitch, from_a);
    } else if c < right && c < 1024.0 {
        extreme =
            c < hot_sample(i - 2u, from_a) && c < hot_sample(i - two, from_a)
            && c < hot_sample(i + two, from_a) && c < hot_sample(i + 1u, from_a)
            && c < hot_sample(i - 1u, from_a) && c < hot_sample(i - pitch, from_a)
            && c < hot_sample(i + pitch, from_a);
    }
    if !extreme {
        return vec2f(c, 0);
    }
    var avg = 0.0;
    avg += hot_sample(i - two - 2u, from_a);
    avg += hot_sample(i - two, from_a);
    avg += hot_sample(i - two + 2u, from_a);
    avg += hot_sample(i - 2u, from_a);
    avg += right;
    avg += hot_sample(i + two - 2u, from_a);
    avg += hot_sample(i + two, from_a);
    avg += hot_sample(i + two + 2u, from_a);
    avg /= 8.0;
    if distance(c, avg) <= 64.0 {
        return vec2f(c, 0);
    }
    let dv = distance(
            hot_sample(i - two, from_a) * hot_sample(i - pitch, from_a),
            hot_sample(i + two, from_a) * hot_sample(i + pitch, from_a),
        );
    let dh = distance(
            hot_sample(i - 2u, from_a) * hot_sample(i - 1u, from_a),
            right * hot_sample(i + 1u, from_a),
        );
    if dv > dh {
        return vec2f((right + hot_sample(i - 2u, from_a)) / 2.0, 1);
    }
    return vec2f((hot_sample(i - two, from_a) + hot_sample(i + two, from_a)) / 2.0, 1);
}
@compute @workgroup_size(16, 16)
fn hot_ab(@builtin(global_invocation_id) pos: vec3u) {
    if pos.x >= args.dims.z || pos.y >= args.dims.w {
        return;
    }
    let i = pos.y * args.dims.z + pos.x;
    if !inside(pos.xy) {
        mb[i] = ma[i];
        return;
    }
    let v = hot_value(i, true);
    mb[i] = v.x;
    if v.y != 0 {
        da[i] |= HOT;
    }
}
@compute @workgroup_size(16, 16)
fn hot_ba(@builtin(global_invocation_id) pos: vec3u) {
    if pos.x >= args.dims.z || pos.y >= args.dims.w {
        return;
    }
    let i = pos.y * args.dims.z + pos.x;
    if !inside(pos.xy) {
        ma[i] = mb[i];
        return;
    }
    let v = hot_value(i, false);
    ma[i] = v.x;
    if v.y != 0 {
        da[i] |= HOT;
    }
}
fn hv_metric(p: vec2i, delta: vec2i) -> f32 {
    let centre = m(p);
    let before = m(p - 2 * delta);
    let after = m(p + 2 * delta);
    let h1 = 2.0 * m(p - delta) / (before + centre);
    let h2 = 2.0 * m(p + delta) / (after + centre);
    var k = distance(h1, h2) * distance(centre * centre, before * after);
    k *= k;
    k *= k;
    k *= k;
    return k * distance(m(p - 3 * delta) * m(p + 3 * delta), m(p - delta) * m(p + delta));
}
@compute @workgroup_size(16, 16)
fn hv_directions(@builtin(global_invocation_id) pos: vec3u) {
    if !inside(pos.xy) {
        return;
    }
    let p = vec2i(pos.xy);
    let i = index(p);
    let dv = hv_metric(p, vec2i(0, 1));
    let dh = hv_metric(p, vec2i(1, 0));
    let strong = select(0u, HVSH, distance(dh, dv) > 256.0);
    da[i] |= select(VER, HOR, dh < dv) | strong;
}
fn checker(p: vec2i) {
    let i = index(p);
    var d = da[i];
    if (d & HVSH) != 0u {
        return;
    }
    let up = da[index(p + vec2i(0, -1))];
    let down = da[index(p + vec2i(0, 1))];
    let left = da[index(p + vec2i(-1, 0))];
    let right = da[index(p + vec2i(1, 0))];
    let nv = ((up & VER) + (down & VER) + (left & VER) + (right & VER)) / VER;
    let nh = ((up & HOR) + (down & HOR) + (left & HOR) + (right & HOR)) / HOR;
    var codir = (left & HOR) != 0u || (right & HOR) != 0u;
    if (d & VER) != 0u {
        codir = (up & VER) != 0u || (down & VER) != 0u;
    }
    if !codir {
        if (d & VER) != 0u && nh > 2u {
            d = (d & ~VER) | HOR;
        }
        if (d & HOR) != 0u && nv > 2u {
            d = (d & ~HOR) | VER;
        }
        da[i] = d;
    }
}
@compute @workgroup_size(16, 16)
fn checker0(@builtin(global_invocation_id) pos: vec3u) {
    if inside(pos.xy) && (pos.x & 1u) == (pos.y & 1u) {
        checker(vec2i(pos.xy));
    }
}
@compute @workgroup_size(16, 16)
fn checker1(@builtin(global_invocation_id) pos: vec3u) {
    if inside(pos.xy) && (pos.x & 1u) != (pos.y & 1u) {
        checker(vec2i(pos.xy));
    }
}
@compute @workgroup_size(16, 16)
fn hv_refine(@builtin(global_invocation_id) pos: vec3u) {
    if pos.x >= args.dims.z || pos.y >= args.dims.w {
        return;
    }
    let p = vec2i(pos.xy);
    let i = index(p);
    var d = da[i];
    if inside(pos.xy) && (d & HVSH) == 0u {
        let all =
            da[index(p + vec2i(0, -1))] & da[index(p + vec2i(0, 1))] & da[index(p + vec2i(-1, 0))]
            & da[index(p + vec2i(1, 0))];
        if (d & VER) != 0u && (all & HOR) != 0u {
            d = (d & ~VER) | HOR;
        }
        if (d & HOR) != 0u && (all & VER) != 0u {
            d = (d & ~HOR) | VER;
        }
    }
    db[i] = d;
}
@compute @workgroup_size(16, 16)
fn make_green(@builtin(global_invocation_id) pos: vec3u) {
    if pos.x >= args.dims.z || pos.y >= args.dims.w {
        return;
    }
    let p = vec2i(pos.xy);
    let i = index(p);
    if !inside(pos.xy) {
        green[i] = 0.5;
        return;
    }
    if colour(p) == 1u {
        green[i] = m(p);
        return;
    }
    var delta = vec2i(1, 0);
    if (db[i] & VER) != 0u {
        delta = vec2i(0, -1);
    }
    let centre = m(p);
    let a = m(p + delta);
    let b = m(p - delta);
    let h1 = 2.0 * a / (m(p + 2 * delta) + centre);
    let h2 = 2.0 * b / (m(p - 2 * delta) + centre);
    var b1 = 1.0 / distance(centre, m(p + 2 * delta));
    var b2 = 1.0 / distance(centre, m(p - 2 * delta));
    b1 *= b1;
    b2 *= b2;
    let value = centre * fma(b1, h1, b2 * h2) / (b1 + b2);
    green[i] = bound(value, a, b, 1u);
}
@compute @workgroup_size(16, 16)
fn diagonal_directions(@builtin(global_invocation_id) pos: vec3u) {
    if !inside(pos.xy) {
        return;
    }
    let p = vec2i(pos.xy);
    let centre = g(p);
    var weight = 1.0;
    if colour(p) != 1u {
        weight = distance(g(p + vec2i(-1, -1)) / 0.5, g(p + vec2i(1, 1)) / 0.5);
    }
    let dl = weight * distance(g(p + vec2i(-1, -1)) * g(p + vec2i(1, 1)), centre * centre);
    let dr = weight * distance(g(p + vec2i(-1, 1)) * g(p + vec2i(1, -1)), centre * centre);
    db[index(p)] |= select(LURD, RULD, dr < dl) | select(0u, DIASH, distance(dl, dr) > 1.4);
}
@compute @workgroup_size(16, 16)
fn diagonal_refine(@builtin(global_invocation_id) pos: vec3u) {
    if pos.x >= args.dims.z || pos.y >= args.dims.w {
        return;
    }
    let p = vec2i(pos.xy);
    let i = index(p);
    var d = db[i];
    if inside(pos.xy) && (d & DIASH) == 0u {
        var all = 0xffffffffu;
        for(var dy = -1; dy <= 1; dy++) {
            for(var dx = -1; dx <= 1; dx++) {
                if dx != 0 || dy != 0 {
                    all &= db[index(p + vec2i(dx, dy))];
                }
            }
        }
        if (d & LURD) != 0u && (all & RULD) != 0u {
            d = (d & ~LURD) | RULD;
        }
        if (d & RULD) != 0u && (all & LURD) != 0u {
            d = (d & ~RULD) | LURD;
        }
    }
    da[i] = d;
}
@compute @workgroup_size(16, 16)
fn red_blue(@builtin(global_invocation_id) pos: vec3u) {
    if pos.x >= args.config.y || pos.y >= args.dims.w {
        return;
    }
    let p = vec2i(i32(pos.x * 2u + (colour(vec2i(0, i32(pos.y))) & 1u)), i32(pos.y));
    let at = pos.y * args.config.y + pos.x;
    if !inside(vec2u(p)) {
        rb[at] = vec2f(0.5);
        return;
    }
    let known = colour(p);
    let missing = known ^ 2u;
    let centre = g(p);
    let dy = select(1, -1, (da[index(p)] & LURD) != 0u);
    let left = p + vec2i(-1, dy);
    let right = p + vec2i(1, -dy);
    let gl = g(left);
    let gr = g(right);
    let t1 = 1.0 / distance(centre, gl);
    let t2 = 1.0 / distance(centre, gr);
    let w1 = t1 * (t1 * t1);
    let w2 = t2 * (t2 * t2);
    let a = m(left);
    let b = m(right);
    let value = bound(centre * (w1 * a / gl + w2 * b / gr) / (w1 + w2), a, b, missing);
    if known == 0u {
        rb[at] = vec2f(m(p), value);
    } else {
        rb[at] = vec2f(value, m(p));
    }
}
fn pixel(p: vec2i) -> vec3f {
    if colour(p) != 1u {
        let v = pair(p);
        return vec3f(v.x, g(p), v.y);
    }
    var delta = vec2i(1, 0);
    if (da[index(p)] & VER) != 0u {
        delta = vec2i(0, -1);
    }
    let centre = g(p);
    let ga = g(p + delta);
    let gb = g(p - delta);
    var w1 = 1.0 / distance(centre, ga);
    var w2 = 1.0 / distance(centre, gb);
    w1 *= w1;
    w2 *= w2;
    let a = pair(p + delta);
    let b = pair(p - delta);
    var rgb = vec3f(0, centre, 0);
    rgb.x = bound(centre * (w1 * a.x / ga + w2 * b.x / gb) / (w1 + w2), a.x, b.x, 0u);
    rgb.z = bound(centre * (w1 * a.y / ga + w2 * b.y / gb) / (w1 + w2), a.y, b.y, 2u);
    return rgb;
}
@compute @workgroup_size(16, 16)
fn output(@builtin(global_invocation_id) pos: vec3u) {
    if !inside(pos.xy) {
        return;
    }
    let p = vec2i(pos.xy);
    let real = p - vec2i(4);
    let known = colour(p);
    var rgb = pixel(p);
    rgb[known] = f32(real_sample(real));
    var codes = vec3u(clamp(rgb, vec3f(0), vec3f(65535)));
    let fourth = codes.y;
    if real.x < 4 || real.y < 4 || u32(real.x + 4) >= args.dims.x
        || u32(real.y + 4) >= args.dims.y {
        var sums = vec3u(0);
        var counts = vec3u(0);
        for(var yy = max(real.y - 1, 0); yy <= min(real.y + 1, i32(args.dims.y) - 1); yy++) {
            for(var xx = max(real.x - 1, 0); xx <= min(real.x + 1, i32(args.dims.x) - 1); xx++) {
                let q = vec2i(xx, yy);
                let c = colour(q);
                sums[c] += real_sample(q);
                counts[c] += 1u;
            }
        }
        for(var c = 0u; c < 3u; c++) {
            if c != known && counts[c] != 0u {
                codes[c] = sums[c] / counts[c];
            }
        }
    }
    result[u32(real.y) * args.dims.x + u32(real.x)] = vec2u(
            codes.x | (codes.y << 16u),
            codes.z | (fourth << 16u),
        );
}
