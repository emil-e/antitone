// Isolated diagnostic: RGB16 storage, extended precision only in shader temporaries.
struct Reference { high: vec4f, low: vec4f, tail: vec4f, anchor_k: vec4f }
struct Parameters {
    gamma_beta: vec4f,
    post: array<vec4f, 3>,
    log_gain: vec4f,
    camera_high: array<vec4f, 3>,
    camera_low: array<vec4f, 3>,
    camera_tail: array<vec4f, 3>,
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
fn expm1_stable(x: f32) -> f32 {
    if abs(x) < 0.001 { return x * (1.0 + x * (0.5 + x * (1.0 / 6.0 + x / 24.0))); }
    return exp(x) - 1.0;
}
fn log_ratio(a: f32, b: f32) -> f32 {
    let z = (a-b)/b;
    if abs(z) < 0.001 { return z*(1.0+z*(-0.5+z*(1.0/3.0-z*0.25))); }
    return log(a)-log(b);
}
fn log_abs_expm1(a: f32) -> f32 {
    if a > 0.0 { return a+log(-expm1_stable(-a)); }
    return log(-expm1_stable(a));
}
fn invert(s: f32, low: f32, tail: f32, c: u32, reference: Reference) -> f32 {
    let delta = fma(1.0, reference.tail[c]-tail, (reference.high[c]-s)+(reference.low[c]-low));
    let z = delta/s;
    var l = log_ratio(reference.high[c],s);
    if abs(z)<0.5 {
        let r=z/(2.0+z); let r2=r*r;
        l=2.0*r*(1.0+r2*(1.0/3.0+r2*(1.0/5.0+r2*(1.0/7.0+r2*(1.0/9.0+r2*(1.0/11.0+r2*(1.0/13.0+r2*(1.0/15.0+r2/17.0))))))));
    }
    if p.gamma_beta.w==1.0 && reference.anchor_k.w==0.0 {
        let a=l/p.gamma_beta[c]; let anchor=reference.anchor_k[c];
        if a>50.0 || anchor>50.0 {
            if a>=0.0 { return exp(a-anchor+p.log_gain.x)*(-expm1_stable(-a))/(-expm1_stable(-anchor)); }
            return expm1_stable(a)*exp(-anchor+p.log_gain.x)/(-expm1_stable(-anchor));
        }
        return expm1_stable(a)/expm1_stable(anchor)*exp(p.log_gain.x);
    }
    let a=p.gamma_beta.w*l/(p.gamma_beta[c]-reference.anchor_k.w*l/log(10.0));
    if a==0.0 { return 0.0; }
    return sign(a)*exp((log_abs_expm1(a)-log_abs_expm1(reference.anchor_k[c]))/p.gamma_beta.w+p.log_gain.x);
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
    var q=vec3f(0.0); var flags=0u;
    for(var c=0u;c<3u;c++) {
        let below=s.high[c]<reference.high.w || (s.high[c]==reference.high.w && (s.low[c]<reference.low.w || (s.low[c]==reference.low.w && s.tail[c]<reference.tail.w)));
        if below { flags|=1u<<c; }
        q[c]=invert(select(s.high[c],reference.high.w,below),select(s.low[c],reference.low.w,below),select(s.tail[c],reference.tail.w,below),c,reference);
    }
    output[id.x]=vec4f(mat3x3f(p.post[0].xyz,p.post[1].xyz,p.post[2].xyz)*q,f32(flags));
}
