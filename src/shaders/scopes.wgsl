struct Parameters {
    size: vec4u,
    luma_gain: vec4f,
    direction_extent: vec4f,
    uv_x: vec4f,
    uv_y: vec4f,
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Parameters;
@group(0) @binding(2) var<storage, read_write> bins: array<atomic<u32>>;
// valid, invalid, off-plot, reserved; then sample totals for each waveform column.
@group(0) @binding(3) var<storage, read_write> totals: array<atomic<u32>>;

fn bin(v: f32) -> u32 {
    return min(u32(max(v, 0.) * 256.), 255u);
}

@compute @workgroup_size(16, 16)
fn reduce(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= p.size.xy) { return; }
    let uv = (vec2f(id.xy) + .5) / vec2f(p.size.xy);
    let mapped = p.uv_x.xy + uv.x * p.uv_x.zw + uv.y * p.uv_y.xy;
    if any(mapped < vec2f(0.)) || any(mapped > vec2f(1.)) {
        atomicAdd(&totals[1], 1u);
        return;
    }
    let source_size = textureDimensions(image);
    let xy = min(vec2u(mapped * vec2f(source_size)), source_size - vec2u(1));
    let rgb = textureLoad(image, vec2i(xy), 0).rgb;
    if !all(rgb >= vec3f(0.)) || !all(rgb <= vec3f(1.)) {
        atomicAdd(&totals[1], 1u);
        return;
    }
    let encoded = select(1.055 * pow(rgb, vec3f(1. / 2.4)) - .055, 12.92 * rgb, rgb <= vec3f(.0031308));
    let y = dot(p.luma_gain.xyz, encoded);
    atomicAdd(&totals[0], 1u);
    let column = bin((f32(id.x) + .5) / f32(p.size.x));
    atomicAdd(&totals[4u + column], 1u);
    if p.size.z == 2u {
        let cb = (encoded.b - y) / (2. * (1. - p.luma_gain.z));
        let cr = (encoded.r - y) / (2. * (1. - p.luma_gain.x));
        let d = p.direction_extent.xy;
        let point = vec2f(dot(d, vec2f(cb, cr)), dot(vec2f(-d.y, d.x), vec2f(cb, cr)));
        let uv = point / (2. * p.direction_extent.z) + .5;
        if any(uv < vec2f(0.)) || any(uv > vec2f(1.)) {
            atomicAdd(&totals[2], 1u);
        } else {
            atomicAdd(&bins[bin(1. - uv.y) * 256u + bin(uv.x)], 1u);
        }
    } else {
        let values = vec4f(encoded, y);
        for (var c = 0u; c < 4u; c++) {
            atomicAdd(&bins[c * 65536u + bin(1. - values[c]) * 256u + column], 1u);
        }
    }
}
