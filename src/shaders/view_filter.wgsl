struct Parameters {
    source: vec2u,
    output: vec2u,
    origin: vec2i,
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
var<uniform> parameters: Parameters;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= parameters.extent) {
        return;
    }

    let pixel = id.xy + parameters.output_origin;
    let start = parameters.start + f32(pixel.x) * parameters.dx + f32(pixel.y) * parameters.dy;
    let points = array<vec2f, 4>(
            start,
            start + parameters.dx,
            start + parameters.dx + parameters.dy,
            start + parameters.dy,
        );
    var low = points[0];
    var high = low;
    for(var i = 1u; i < 4u; i++) {
        low = min(low, points[i]);
        high = max(high, points[i]);
    }

    let last = vec2i(parameters.source) - vec2i(1);
    let lower = max(clamp(vec2i(floor(low)), vec2i(0), last), parameters.origin);
    let upper = min(
            clamp(vec2i(ceil(high)), vec2i(1), last + vec2i(1)),
            parameters.origin + vec2i(parameters.tile),
        );
    if any(lower >= upper) {
        return;
    }

    var sum = vec3f(0.);
    var flags = 0u;
    for(var y = lower.y; y < upper.y; y++) {
        for(var x = lower.x; x < upper.x; x++) {
            let weight = extended_overlap(
                    start,
                    vec2f(f32(x), f32(y)),
                    parameters.dx,
                    parameters.dy,
                    vec2f(0.),
                    vec2f(parameters.source),
                );
            if weight > 0. {
                let sample = textureLoad(image, vec2i(x, y) - vec2i(parameters.origin), 0);
                sum += sample.rgb * weight;
                // Low three bits retain the union of original inversion-floor flags; bit 3 is coverage.
                flags |= u32(sample.a) | 8u;
            }
        }
    }

    let index = pixel.x + pixel.y * parameters.output.x;
    let previous = sums[index];
    sums[index] = vec4f(
            previous.rgb + sum / abs(cross(parameters.dx, parameters.dy)),
            f32(u32(previous.a) | flags),
        );
}
