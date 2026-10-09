struct Pair { h: f32, l: f32 }
fn pair(a: f32) -> Pair { return Pair(a, 0.0); }
fn pn(a: Pair) -> Pair { return Pair(-a.h, -a.l); }
fn pa(a: Pair, b: Pair) -> Pair {
    let s = two_sum(a.h, b.h);
    let e = fma(1.0, fma(1.0, a.l, b.l), s.y);
    let q = two_sum(s.x, e);
    return Pair(q.x, q.y);
}
fn ps(a: Pair, b: Pair) -> Pair { return pa(a, pn(b)); }
fn pm(a: Pair, b: Pair) -> Pair {
    let h = fma(a.h, b.h, 0.0);
    var e = fma(a.h, b.h, -h);
    e = fma(a.h, b.l, e);
    e = fma(a.l, b.h, e);
    e = fma(a.l, b.l, e);
    let q = two_sum(h, e);
    return Pair(q.x, q.y);
}
fn pd(a: Pair, b: Pair) -> Pair {
    let h = a.h / b.h;
    let r = ps(a, pm(b, pair(h)));
    let l = fma(1.0, r.h, r.l) / b.h;
    let q = two_sum(h, l);
    return Pair(q.x, q.y);
}
fn pt(a: Triple) -> Pair { let q = two_sum(a.h, fma(1.0, a.l, a.t)); return Pair(q.x, q.y); }
fn log_series(z: Pair) -> Pair {
    let r = pd(z, pa(pair(2.0), z));
    let r2 = pm(r, r);
    var term = r;
    var sum = r;
    for (var i = 1u; i < 15u; i++) {
        term = pm(term, r2);
        sum = pa(sum, pd(term, pair(f32(2u * i + 1u))));
    }
    return pm(pair(2.0), sum);
}
fn plog(x: Pair) -> Pair {
    let bits = bitcast<u32>(x.h);
    let exponent = (bits >> 23u) & 255u;
    var n = i32(exponent) - 127;
    var m = Pair(bitcast<f32>((bits & 0x7fffffu) | (127u << 23u)), x.l * bitcast<f32>((254u - exponent) << 23u));
    if m.h > 1.4142135623730951 { m = pm(m, pair(0.5)); n += 1; }
    return pa(log_series(ps(m, pair(1.0))), pm(pair(f32(n)), Pair(0.6931471824645996, -1.9046542121259336e-9)));
}
fn plog_ratio(base: Triple, sample: Triple) -> Pair {
    let delta = add(base, Triple(-sample.h, -sample.l, -sample.t));
    let z = pd(pt(delta), pt(sample));
    if abs(z.h) < 0.5 { return log_series(z); }
    return ps(plog(pt(base)), plog(pt(sample)));
}
fn pexp(x: Pair) -> Pair {
    let n = round(x.h / 0.6931471824645996);
    let r = ps(x, pm(pair(n), Pair(0.6931471824645996, -1.9046542121259336e-9)));
    var term = pair(1.0);
    var sum = term;
    for (var i = 1u; i < 18u; i++) {
        term = pd(pm(term, r), pair(f32(i)));
        sum = pa(sum, term);
    }
    return pm(sum, pair(exp2(n)));
}
fn pexpm1(x: Pair) -> Pair {
    if abs(x.h) < 0.5 {
        var term = x;
        var sum = term;
        for (var i = 2u; i < 18u; i++) {
            term = pd(pm(term, x), pair(f32(i)));
            sum = pa(sum, term);
        }
        return sum;
    }
    return ps(pexp(x), pair(1.0));
}
fn plog_abs_expm1(x: Pair) -> Pair {
    if x.h > 50.0 { return x; }
    if x.h > 0.0 { return pa(x, plog(pn(pexpm1(pn(x))))); }
    return plog(pn(pexpm1(x)));
}
fn invert(s: f32, low: f32, tail: f32, c: u32, reference: Reference) -> Pair {
    let l = plog_ratio(Triple(reference.high[c],reference.low[c],reference.tail[c]), Triple(s,low,tail));
    let beta = Pair(p.gamma_beta.w, p.gamma_beta_low.w);
    let k = Pair(reference.anchor_k.w, reference.anchor_low.w);
    let gamma = Pair(p.gamma_beta[c],p.gamma_beta_low[c]);
    let density = pd(l, Pair(2.3025851249694824, -3.1975435632602967e-8));
    let a = pd(pm(beta,l), ps(gamma,pm(k,density)));
    if a.h == 0.0 && a.l == 0.0 { return pair(0.0); }
    let anchored = pd(ps(plog_abs_expm1(a), Pair(reference.anchor_k[c],reference.anchor_low[c])), beta);
    let q = pexp(pa(anchored, Pair(p.log_gain.x,p.log_gain.y)));
    return pm(pair(sign(a.h)), q);
}
