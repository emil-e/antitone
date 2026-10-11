struct CameraMatrix {
    high: array<vec4f, 3>,
    low: array<vec4f, 3>,
    tail: array<vec4f, 3>,
}
@group(0) @binding(3) var<uniform> camera_matrix: CameraMatrix;

// Keep conversion precision in temporaries, not in the retained RGB16 source buffer.
struct Triple { h: f32, l: f32, t: f32 }
struct ConvertedScan { high: vec3f, low: vec3f, tail: vec3f }
fn two_sum(a: f32, b: f32) -> vec2f {
    let s = fma(1.0, a, b);
    let v = fma(-1.0, a, s);
    let u = fma(-1.0, v, s);
    return vec2f(s, fma(1.0, fma(1.0, a, -u), fma(1.0, b, -v)));
}
fn add_term(a: Triple, b: f32) -> Triple {
    let x = two_sum(a.h, b);
    let y = two_sum(a.l, x.y);
    let z = two_sum(a.t, y.y);
    let n = two_sum(y.x, z.x);
    let h = two_sum(x.x, n.x);
    let l = two_sum(h.y, fma(1.0, n.y, z.y));
    return Triple(h.x, l.x, l.y);
}
fn add(a: Triple, b: Triple) -> Triple {
    return add_term(add_term(add_term(a, b.h), b.l), b.t);
}
fn product(a: f32, b: f32) -> Triple {
    let h = fma(a, b, 0.0);
    return Triple(h, fma(a, b, -h), 0.0);
}
fn multiply(a: Triple, b: Triple) -> Triple {
    var q = product(a.h, b.h);
    q = add(q, product(a.h, b.l));
    q = add(q, product(a.l, b.h));
    q = add(q, product(a.h, b.t));
    q = add(q, product(a.l, b.l));
    q = add(q, product(a.t, b.h));
    q = add(q, product(a.l, b.t));
    q = add(q, product(a.t, b.l));
    return add(q, product(a.t, b.t));
}
// Match the scalar f64 rounding between normalization, products and sums.
fn round53(a: Triple) -> Triple {
    if a.h == 0.0 { return a; }
    // Rescale small cancellations so the rounding quantum is a normal f32.
    let scale = select(1.0, 18446744073709551616.0, abs(a.h) <= 0x1p-74f);
    let value = Triple(a.h * scale, a.l * scale, a.t * scale);
    let bits = bitcast<u32>(abs(value.h));
    var e = (bits >> 23u) & 255u;
    let correction = select(value.l, value.t, value.l == 0.0);
    if (bits & 0x7fffffu) == 0u && sign(value.h) * correction < 0.0 { e -= 1u; }
    let step = bitcast<f32>((e - 52u) << 23u);
    let units = value.l / step;
    let whole = floor(units);
    let remainder = fma(1.0, value.t / step, fma(1.0, units, -whole));
    let lower = floor(remainder);
    let fraction = fma(1.0, remainder, -lower);
    let odd = ((i32(whole) + i32(lower)) & 1) != 0;
    let upper = fraction > 0.5 || (fraction == 0.5 && odd);
    let rounded = lower + select(0.0, 1.0, upper);
    let result = add_term(Triple(value.h, whole * step, 0.0), rounded * step);
    return Triple(result.h / scale, result.l / scale, result.t / scale);
}
fn normalized(code: u32) -> Triple {
    let value = f32(code);
    let h = value / 65535.0;
    let rem = fma(-h, 65535.0, value);
    let l = rem / 65535.0;
    let t = fma(-l, 65535.0, rem) / 65535.0;
    return round53(Triple(h, l, t));
}
fn convert_camera(code: vec2u) -> ConvertedScan {
    let x = normalized(code.x & 65535u);
    let y = normalized(code.x >> 16u);
    let z = normalized(code.y & 65535u);
    return convert_components(x, y, z);
}
fn convert_components(x: Triple, y: Triple, z: Triple) -> ConvertedScan {
    return ConvertedScan(vec3f(x.h, y.h, z.h), vec3f(x.l, y.l, z.l), vec3f(x.t, y.t, z.t));
}
fn camera_source_to_ap1(scan: ConvertedScan) -> ConvertedScan {
    var working: ConvertedScan;
    for (var c = 0u; c < 3u; c++) {
        var sum = Triple(0.0, 0.0, 0.0);
        for (var j = 0u; j < 3u; j++) {
            let m = Triple(camera_matrix.high[j][c], camera_matrix.low[j][c], camera_matrix.tail[j][c]);
            let s = Triple(scan.high[j], scan.low[j], scan.tail[j]);
            sum = round53(add(sum, round53(multiply(m, s))));
        }
        working.high[c] = sum.h; working.low[c] = sum.l; working.tail[c] = sum.t;
    }
    return working;
}
