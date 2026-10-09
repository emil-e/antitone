struct Parameters {
    dmin_floor: vec4f,
    gamma_beta: vec4f,
    anchor_k: vec4f,
    post: array<vec4f, 3>,
    log_gain: vec4f,
}
@group(0) @binding(0) var<storage, read> scans: array<vec4f>;
@group(0) @binding(1) var<storage, read_write> positives: array<vec4f>;
@group(0) @binding(2) var<uniform> p: Parameters;
fn expm1_stable(x: f32) -> f32 {
    if abs(x) < 0.001 { return x * (1.0 + x * (0.5 + x * (1.0 / 6.0 + x / 24.0))); }
    return exp(x) - 1.0;
}
fn log_ratio(a: f32, b: f32) -> f32 {
    let z = (a - b) / b;
    if abs(z) < 0.001 { return z * (1.0 + z * (-0.5 + z * (1.0 / 3.0 - z * 0.25))); }
    return log(a) - log(b);
}
fn log_abs_expm1(a: f32) -> f32 {
    if a > 0.0 { return a + log(-expm1_stable(-a)); }
    return log(-expm1_stable(a));
}
fn invert(s: f32, c: u32) -> f32 {
    let l = log_ratio(p.dmin_floor[c], s);
    if p.gamma_beta.w == 1.0 && p.anchor_k.w == 0.0 {
        let a=l/p.gamma_beta[c]; let anchor=p.anchor_k[c];
        if a > 50.0 || anchor > 50.0 {
            if a >= 0.0 { return exp(a-anchor+p.log_gain.x)*(-expm1_stable(-a))/(-expm1_stable(-anchor)); }
            return expm1_stable(a)*exp(-anchor+p.log_gain.x)/(-expm1_stable(-anchor));
        }
        return expm1_stable(a)/expm1_stable(anchor)*exp(p.log_gain.x);
    }
    let d = l / log(10.0);
    let a = p.gamma_beta.w * l / (p.gamma_beta[c] - p.anchor_k.w * d);
    if a == 0.0 { return 0.0; }
    return sign(a) * exp((log_abs_expm1(a)-log_abs_expm1(p.anchor_k[c]))/p.gamma_beta.w+p.log_gain.x);
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&scans) { return; }
    var q=vec3f(0.0); var flags=0u;
    for(var c=0u;c<3u;c++) {
        if scans[id.x][c] < p.dmin_floor.w { flags |= 1u<<c; }
        q[c]=invert(max(scans[id.x][c],p.dmin_floor.w),c);
    }
    let matrix=mat3x3f(p.post[0].xyz,p.post[1].xyz,p.post[2].xyz);
    positives[id.x]=vec4f(matrix*q,f32(flags));
}
