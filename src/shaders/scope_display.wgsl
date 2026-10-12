struct Vertex { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Vertex {
    let xy = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.))[i];
    return Vertex(vec4f(xy, 0., 1.), vec2f(xy.x * .5 + .5, .5 - xy.y * .5));
}
struct Parameters {
    size: vec4u,
    luma_gain: vec4f,
    direction_extent: vec4f,
}
@group(0) @binding(0) var<storage, read> bins: array<u32>;
@group(0) @binding(1) var<storage, read> totals: array<u32>;
@group(0) @binding(2) var<uniform> p: Parameters;

fn density(count: u32, total: u32, scale: f32) -> f32 {
    return clamp(log2(1. + f32(count) / max(f32(total), 1.) * scale) * .18, 0., 1.);
}
@fragment fn fragment(@location(0) uv: vec2f) -> @location(0) vec4f {
    let xy = min(vec2u(max(uv, vec2f(0.)) * 256.), vec2u(255));
    let index = xy.y * 256u + xy.x;
    var trace = vec3f(0.);
    if p.size.z == 2u {
        // Amplification shrinks chroma bin area by gain²; display density, not bin occupancy.
        let scale = 10000. * p.luma_gain.w * p.luma_gain.w;
        trace = vec3f(.93, .96, 1.) * density(bins[index], totals[0], scale);
    } else if p.size.z == 1u {
        trace = vec3f(density(bins[3u * 65536u + index], totals[4u + xy.x], 1000.));
    } else {
        trace = vec3f(
            density(bins[index], totals[4u + xy.x], 1000.),
            density(bins[65536u + index], totals[4u + xy.x], 1000.),
            density(bins[131072u + index], totals[4u + xy.x], 1000.));
    }
    if totals[1] != 0u {
        trace *= .25;
    }
    return vec4f(max(vec3f(.025), trace), 1.);
}
