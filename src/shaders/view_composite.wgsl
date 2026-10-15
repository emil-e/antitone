// Both inputs are the same accepted grade/target, already rendered exactly once.
struct Parameters {
    output: vec4u,
    detail_x: vec4f, detail_y: vec4f,
    underlay_x: vec4f, underlay_y: vec4f,
}
@group(0) @binding(0) var detail: texture_2d<f32>;
@group(0) @binding(1) var underlay: texture_2d<f32>;
@group(0) @binding(2) var<uniform> p: Parameters;
@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let v = array<vec2f,3>(vec2f(-1.,-1.), vec2f(3.,-1.), vec2f(-1.,3.))[i];
    return vec4f(v,0.,1.);
}
// Premultiplied linear samples keep unsupported pixels out of the colour interpolation.
fn sample_image(image: texture_2d<f32>, uv: vec2f) -> vec4f {
    if any(uv < vec2f(0.)) || any(uv >= vec2f(1.)) { return vec4f(0.); }
    let size = vec2i(textureDimensions(image));
    let at = uv * vec2f(size) - vec2f(0.5);
    let lo = vec2i(floor(at));
    let f = fract(at);
    var result = vec4f(0.);
    for (var y = 0; y < 2; y++) { for (var x = 0; x < 2; x++) {
        let weight = select(1.-f.x, f.x, x==1) * select(1.-f.y, f.y, y==1);
        let value = textureLoad(image, clamp(lo+vec2i(x,y), vec2i(0), size-vec2i(1)), 0);
        result += vec4f(value.rgb * value.a, value.a) * weight;
    } }
    return result;
}
@fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let uv = position.xy / vec2f(p.output.xy);
    let d = sample_image(detail, p.detail_x.xy + uv.x*p.detail_x.zw + uv.y*p.detail_y.xy);
    // Full-detail pixels bypass the underlay, including perfectly black valid pixels.
    if d.a >= 1. { return vec4f(d.rgb, 1.); }
    let b = sample_image(underlay, p.underlay_x.xy + uv.x*p.underlay_x.zw + uv.y*p.underlay_y.xy);
    let value = d + b * (1.-d.a);
    if value.a <= 0. { return vec4f(0.); }
    return vec4f(value.rgb / value.a, 1.);
}
