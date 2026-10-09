// Isolated diagnostic: RGB16 storage, extended precision only in shader temporaries.
struct Reference { high: vec4f, low: vec4f, tail: vec4f, anchor_k: vec4f, anchor_low: vec4f }
struct Parameters {
    gamma_beta: vec4f,
    post: array<vec4f, 3>,
    log_gain: vec4f,
    camera_high: array<vec4f, 3>,
    camera_low: array<vec4f, 3>,
    camera_tail: array<vec4f, 3>,
    gamma_beta_low: vec4f,
    post_low: array<vec4f, 3>,
}
@group(0) @binding(0) var<storage, read> scans: array<vec2u>;
@group(0) @binding(1) var<storage, read_write> output: array<vec4f>;
@group(0) @binding(2) var<uniform> p: Parameters;
@group(0) @binding(3) var<storage, read> references: array<Reference>;
struct Triple { h: f32, l: f32, t: f32 }
struct Scan { high: vec3f, low: vec3f, tail: vec3f }
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
// Match the scalar's f64 rounding after each source-conversion operation.
// These normalized camera values/matrix products do not enter f32 subnormal range.
fn round53(a: Triple) -> Triple {
    if a.h == 0.0 { return a; }
    let bits = bitcast<u32>(abs(a.h));
    var e = (bits >> 23u) & 255u;
    let correction = select(a.l, a.t, a.l == 0.0);
    if (bits & 0x7fffffu) == 0u && sign(a.h) * correction < 0.0 { e -= 1u; }
    let step = bitcast<f32>((e - 52u) << 23u);
    let units = a.l / step;
    let whole = floor(units);
    let remainder = fma(1.0, a.t / step, fma(1.0, units, -whole));
    let lower = floor(remainder);
    let fraction = fma(1.0, remainder, -lower);
    let odd = ((i32(whole) + i32(lower)) & 1) != 0;
    let upper = fraction > 0.5 || (fraction == 0.5 && odd);
    let rounded = lower + select(0.0, 1.0, upper);
    return add_term(Triple(a.h, whole * step, 0.0), rounded * step);
}
fn normalized(code: u32) -> Triple {
    let value = f32(code);
    let h = value / 65535.0;
    let rem = fma(-h, 65535.0, value);
    let l = rem / 65535.0;
    let t = fma(-l, 65535.0, rem) / 65535.0;
    return round53(Triple(h, l, t));
}
fn convert(code: vec2u) -> Scan {
    let x = normalized(code.x & 65535u);
    let y = normalized(code.x >> 16u);
    let z = normalized(code.y & 65535u);
    var s: Scan;
    for (var c = 0u; c < 3u; c++) {
        let mx = Triple(p.camera_high[0][c], p.camera_low[0][c], p.camera_tail[0][c]);
        let my = Triple(p.camera_high[1][c], p.camera_low[1][c], p.camera_tail[1][c]);
        let mz = Triple(p.camera_high[2][c], p.camera_low[2][c], p.camera_tail[2][c]);
        let a = round53(multiply(mx, x));
        let b = round53(multiply(my, y));
        let d = round53(multiply(mz, z));
        let q = round53(add(round53(add(a, b)), d));
        s.high[c] = q.h; s.low[c] = q.l; s.tail[c] = q.t;
    }
    return s;
}
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
@compute @workgroup_size(64)
fn conversion_only(@builtin(global_invocation_id) id: vec3u) {
    if id.x>=arrayLength(&scans) { return; }
    let s=convert(scans[id.x]);
    output[3u*id.x]=vec4f(s.high,f32(arrayLength(&references)));
    output[3u*id.x+1u]=vec4f(s.low,0.0);
    output[3u*id.x+2u]=vec4f(s.tail,0.0);
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x>=arrayLength(&scans) { return; }
    let s=convert(scans[id.x]);
    let reference=references[select(id.x,0u,arrayLength(&references)==1u)];
    var q: array<Pair,3>; var flags=0u;
    for(var c=0u;c<3u;c++) {
        let below=s.high[c]<reference.high.w || (s.high[c]==reference.high.w && (s.low[c]<reference.low.w || (s.low[c]==reference.low.w && s.tail[c]<reference.tail.w)));
        if below { flags|=1u<<c; }
        q[c]=invert(select(s.high[c],reference.high.w,below),select(s.low[c],reference.low.w,below),select(s.tail[c],reference.tail.w,below),c,reference);
    }
    var result=vec3f(0.0);
    for (var c=0u;c<3u;c++) {
        let x=pm(Pair(p.post[0][c],p.post_low[0][c]),q[0]);
        let y=pm(Pair(p.post[1][c],p.post_low[1][c]),q[1]);
        let z=pm(Pair(p.post[2][c],p.post_low[2][c]),q[2]);
        let sum=pa(pa(x,y),z);
        result[c]=fma(1.0,sum.h,sum.l);
    }
    output[id.x]=vec4f(result,f32(flags));
}
