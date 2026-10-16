struct Parameters {
    source: vec2u,
    output: vec2u,
    origin: vec2u,
    tile: vec2u,
    start: vec2f,
    dx: vec2f,
    dy: vec2f,
    padding: vec2f,
    output_origin: vec2u,
    extent: vec2u,
}
@group(0) @binding(0)
var image: texture_2d<f32>;
@group(0) @binding(1)
var<storage, read_write> sums: array<vec4f>;
@group(0) @binding(2)
var<uniform> p: Parameters;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= p.extent) {
        return;
    }
    let pixel = id.xy + p.output_origin;
    let start = p.start + f32(pixel.x) * p.dx + f32(pixel.y) * p.dy;
    let points = array<vec2f, 4>(start, start + p.dx, start + p.dx + p.dy, start + p.dy);
    var low = points[0];
    var high = low;
    for(var i = 1u; i < 4u; i++) {
        low = min(low, points[i]);
        high = max(high, points[i]);
    }
    let lo = max(vec2i(floor(low)), vec2i(p.origin));
    let hi = min(min(vec2i(ceil(high)), vec2i(p.origin + p.tile)), vec2i(p.source));
    if any(lo >= hi) {
        return;
    }
    var sum = vec3f(0.);
    for(var y = lo.y; y < hi.y; y++) {
        for(var x = lo.x; x < hi.x; x++) {
            let weight = overlap(start, vec2f(f32(x), f32(y)), p.dx, p.dy);
            if weight > 0. {
                sum += textureLoad(image, vec2i(x, y) - vec2i(p.origin), 0).rgb * weight;
            }
        }
    }
    let index = pixel.x + pixel.y * p.output.x;
    sums[index] += vec4f(sum / abs(cross(p.dx, p.dy)), 0.);
}
