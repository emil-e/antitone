// Common linear-pixel reconstruction after original processing and any output transform.
struct Mapping {
    sizes: vec4u,
    origin: vec4f,
    dx: vec4f,
    dy: vec4f,
    space: vec4u,
}
@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var<storage, read> working: array<vec4f>;
@group(0) @binding(2)
var<storage, read_write> destination: array<vec4f>;
@group(0) @binding(3)
var<uniform> mapping: Mapping;
@group(0) @binding(4)
var<storage, read> support: array<u32>;

struct Polygon {
    points: array<vec2f, 8>,
    count: u32,
}

fn clip(input: Polygon, axis: u32, edge: f32, greater: bool) -> Polygon {
    var output: Polygon;
    if input.count == 0u {
        return output;
    }
    var a = input.points[input.count - 1u];
    var ai = select(a[axis] <= edge, a[axis] >= edge, greater);
    for(var i = 0u; i < input.count; i++) {
        let b = input.points[i];
        let bi = select(b[axis] <= edge, b[axis] >= edge, greater);
        if ai != bi && a[axis] != edge && b[axis] != edge {
            let t = (edge - a[axis]) / (b[axis] - a[axis]);
            var p = a + t * (b - a);
            p[axis] = edge;
            output.points[output.count] = p;
            output.count++;
        }
        if bi {
            output.points[output.count] = b;
            output.count++;
        }
        a = b;
        ai = bi;
    }
    return output;
}

fn area(corners: array<vec2f, 4>, cell: vec2i) -> f32 {
    var polygon: Polygon;
    polygon.count = 4u;
    // Cell-local clipping/triangle areas avoid cancellation of large image-space products.
    for(var i = 0u; i < 4u; i++) {
        polygon.points[i] = corners[i] - vec2f(cell);
    }
    polygon = clip(polygon, 0u, 0.0, true);
    polygon = clip(polygon, 0u, 1.0, false);
    polygon = clip(polygon, 1u, 0.0, true);
    polygon = clip(polygon, 1u, 1.0, false);
    var sum = 0.0;
    for(var i = 1u; i + 1u < polygon.count; i++) {
        let a = polygon.points[i] - polygon.points[0];
        let b = polygon.points[i + 1u] - polygon.points[0];
        sum += a.x * b.y - a.y * b.x;
    }
    return abs(sum) * 0.5;
}

fn mapped(p: vec2u) -> vec2f {
    return mapping.origin.xy + f32(p.x) * mapping.dx.xy + f32(p.y) * mapping.dy.xy;
}

fn finite(v: vec3f) -> bool {
    return all((bitcast<vec3u>(v) & vec3u(0x7f800000u)) != vec3u(0x7f800000u));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= mapping.sizes.zw) {
        return;
    }
    let index = id.y * mapping.sizes.z + id.x;
    let corners = array<vec2f, 4>(
            mapped(id.xy),
            mapped(id.xy + vec2u(1, 0)),
            mapped(id.xy + vec2u(1)),
            mapped(id.xy + vec2u(0, 1)),
        );
    var lo = corners[0];
    var hi = corners[0];
    for(var i = 1u; i < 4u; i++) {
        lo = min(lo, corners[i]);
        hi = max(hi, corners[i]);
    }
    // The CPU verifies exact support lies inside the active area. At the physical source edge,
    // native mapping roundoff is intersected with real cells and weights are renormalized;
    // no replicated/blank border pixels or shifted crop are introduced.
    let first = max(vec2i(floor(lo)), vec2i(0));
    let end = min(vec2i(ceil(hi)), vec2i(mapping.sizes.xy));
    var linear = vec4f(0.0);
    var residual = vec4f(0.0);
    var error = 0.0;
    for(var y = first.y; y < end.y; y++) {
        for(var x = first.x; x < end.x; x++) {
            let source_index = u32(y) * mapping.sizes.x + u32(x);
            if mapping.space.y != 0u
                && (support[source_index / 32u] & (1u << (source_index % 32u))) == 0u {
                continue;
            }
            let cell = vec2i(x, y);
            let weight = area(corners, cell);
            if weight == 0.0 {
                continue;
            }
            if !(weight > 0.0 && weight <= 1.00001) {
                error = -3.0;
            }
            let ap1 = working[source_index];
            var pixel = ap1;
            if mapping.space.x == 0u {
                pixel = textureLoad(source, cell, 0);
            }
            if !finite(ap1.rgb) || !finite(pixel.rgb) {
                error = -1.0;
            }
            if pixel.a < 0.0 {
                error = -2.0;
            }
            // Signed HDR can cancel large terms. Keep multiplication/addition residuals
            // in the same accumulator for every output, not an EXR-only numeric path.
            // Accumulate the normalization with the same arithmetic to conserve constants.
            let value = vec4f(pixel.rgb, 1.0);
            let weighted = weight * value;
            let product_error = fma(vec4f(weight), value, -weighted);
            let ordered = abs(linear) >= abs(weighted);
            let large = select(weighted, linear, ordered);
            let small = select(linear, weighted, ordered);
            let sum = fma(vec4f(1.0), large, small);
            residual += fma(vec4f(1.0), small, fma(vec4f(1.0), large, -sum)) + product_error;
            linear = sum;
        }
    }
    let combined = linear + residual;
    let total = combined.w;
    let expected = abs(mapping.dx.x * mapping.dy.y - mapping.dx.y * mapping.dy.x);
    if !(total > 0.0) || abs(total - expected) > 0.0001 * expected {
        error = -3.0;
    }
    if error < 0.0 {
        destination[index] = vec4f(0.0, 0.0, 0.0, error);
    } else {
        destination[index] = vec4f(combined.rgb / total, total);
    }
}
