// Compensated f32 pairs keep response and AP1 cancellation precision in temporaries.
struct Pair { h: f32, l: f32 }
const LN2_PAIR: Pair = Pair(0.6931471824645996, -1.9046542121259336e-9);
const LN10_PAIR: Pair = Pair(2.3025851249694824, -3.1975435632602967e-8);
fn pair(a: f32) -> Pair { return Pair(a, 0.0); }
fn pair_neg(a: Pair) -> Pair { return Pair(-a.h, -a.l); }
fn pair_add(a: Pair, b: Pair) -> Pair {
    let s = two_sum(a.h, b.h);
    let e = fma(1.0, fma(1.0, a.l, b.l), s.y);
    let q = two_sum(s.x, e);
    return Pair(q.x, q.y);
}
fn pair_sub(a: Pair, b: Pair) -> Pair { return pair_add(a, pair_neg(b)); }
fn pair_mul(a: Pair, b: Pair) -> Pair {
    let h = fma(a.h, b.h, 0.0);
    var e = fma(a.h, b.h, -h);
    e = fma(a.h, b.l, e);
    e = fma(a.l, b.h, e);
    e = fma(a.l, b.l, e);
    let q = two_sum(h, e);
    return Pair(q.x, q.y);
}
fn pair_div(a: Pair, b: Pair) -> Pair {
    let h = a.h / b.h;
    let r = pair_sub(a, pair_mul(b, pair(h)));
    let l = fma(1.0, r.h, r.l) / b.h;
    let q = two_sum(h, l);
    return Pair(q.x, q.y);
}
fn pair_from_triple(a: Triple) -> Pair {
    let q = two_sum(a.h, fma(1.0, a.l, a.t));
    return Pair(q.x, q.y);
}
fn pair_log1p(z: Pair) -> Pair {
    let r = pair_div(z, pair_add(pair(2.0), z));
    let r2 = pair_mul(r, r);
    var term = r;
    var sum = r;
    for (var i = 1u; i < 15u; i++) {
        term = pair_mul(term, r2);
        sum = pair_add(sum, pair_div(term, pair(f32(2u * i + 1u))));
    }
    return pair_mul(pair(2.0), sum);
}
fn pair_log(x: Pair) -> Pair {
    let bits = bitcast<u32>(x.h);
    let exponent = (bits >> 23u) & 255u;
    var n = i32(exponent) - 127;
    var m = Pair(bitcast<f32>((bits & 0x7fffffu) | (127u << 23u)), x.l * bitcast<f32>((254u - exponent) << 23u));
    if m.h > 1.4142135623730951 { m = pair_mul(m, pair(0.5)); n += 1; }
    return pair_add(pair_log1p(pair_sub(m, pair(1.0))), pair_mul(pair(f32(n)), LN2_PAIR));
}
fn pair_log_ratio(base: Triple, sample: Triple) -> Pair {
    // Subtract all source/reference parts before narrowing the small near-base difference.
    let delta = add(base, Triple(-sample.h, -sample.l, -sample.t));
    let z = pair_div(pair_from_triple(delta), pair_from_triple(sample));
    if abs(z.h) < 0.5 { return pair_log1p(z); }
    return pair_sub(pair_log(pair_from_triple(base)), pair_log(pair_from_triple(sample)));
}
fn pair_exp(x: Pair) -> Pair {
    let n = round(x.h / LN2_PAIR.h);
    let r = pair_sub(x, pair_mul(pair(n), LN2_PAIR));
    var term = pair(1.0);
    var sum = term;
    for (var i = 1u; i < 18u; i++) {
        term = pair_div(pair_mul(term, r), pair(f32(i)));
        sum = pair_add(sum, term);
    }
    return pair_mul(sum, pair(exp2(n)));
}
fn pair_expm1(x: Pair) -> Pair {
    if abs(x.h) < 0.5 {
        var term = x;
        var sum = term;
        for (var i = 2u; i < 18u; i++) {
            term = pair_div(pair_mul(term, x), pair(f32(i)));
            sum = pair_add(sum, term);
        }
        return sum;
    }
    return pair_sub(pair_exp(x), pair(1.0));
}
fn pair_log_abs_expm1(x: Pair) -> Pair {
    if x.h > 50.0 { return x; }
    if x.h > 0.0 { return pair_add(x, pair_log(pair_neg(pair_expm1(pair_neg(x))))); }
    return pair_log(pair_neg(pair_expm1(x)));
}
fn invert_film(sample: Triple, c: u32) -> Pair {
    let base = Triple(parameters.dmin_floor[c], parameters.dmin_low[c], parameters.dmin_tail[c]);
    let l = pair_log_ratio(base, sample);
    let beta = Pair(parameters.shape.x, parameters.shape.z);
    let k = Pair(parameters.shape.y, parameters.shape.w);
    let gamma = Pair(parameters.gamma_mode[c], parameters.gamma_low[c]);
    let density = pair_div(l, LN10_PAIR);
    let a = pair_div(pair_mul(beta, l), pair_sub(gamma, pair_mul(k, density)));
    if a.h == 0.0 && a.l == 0.0 { return pair(0.0); }
    let anchor = Pair(parameters.anchor[c], parameters.anchor_low[c]);
    let anchored = pair_div(pair_sub(pair_log_abs_expm1(a), anchor), beta);
    let gain = Pair(parameters.anchor.w, parameters.anchor_low.w);
    return pair_mul(pair(sign(a.h)), pair_exp(pair_add(anchored, gain)));
}
fn process_film(scan: ConvertedScan) -> vec4f {
    var q: array<Pair, 3>;
    var flags = 0u;
    let floor = Triple(parameters.dmin_floor.w, parameters.dmin_low.w, parameters.dmin_tail.w);
    for (var c = 0u; c < 3u; c++) {
        var s = Triple(scan.high[c], scan.low[c], scan.tail[c]);
        let below = s.h < floor.h || (s.h == floor.h && (s.l < floor.l || (s.l == floor.l && s.t < floor.t)));
        if below { flags |= 1u << c; s = floor; }
        q[c] = invert_film(s, c);
    }
    var result = vec3f(0.0);
    for (var c = 0u; c < 3u; c++) {
        let x = pair_mul(Pair(parameters.post[0][c], parameters.post_low[0][c]), q[0]);
        let y = pair_mul(Pair(parameters.post[1][c], parameters.post_low[1][c]), q[1]);
        let z = pair_mul(Pair(parameters.post[2][c], parameters.post_low[2][c]), q[2]);
        let sum = pair_add(pair_add(x, y), z);
        result[c] = fma(1.0, sum.h, sum.l);
    }
    return vec4f(result, f32(flags));
}
