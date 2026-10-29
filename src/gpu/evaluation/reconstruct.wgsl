struct Geometry {
    sizes: vec4u,
    origins: vec4u,
    source_bounds: vec4u,
    mapping: vec4f,
}
@group(0) @binding(21)
var<storage, read_write> output: array<vec4f>;
@group(0) @binding(22)
var<uniform> geometry: Geometry;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2f(f32(index % size.x), f32(index / size.x)) + vec2f(geometry.origins.zw);
    let lo = geometry.mapping.xy + xy * geometry.mapping.zw;
    let hi = geometry.mapping.xy + (xy + vec2f(1.0)) * geometry.mapping.zw;
    let first_cell = vec2i(geometry.source_bounds.xy);
    let last_cell = vec2i(geometry.source_bounds.zw) - vec2i(1);
    let first = clamp(vec2i(floor(lo)), first_cell, last_cell);
    let end = clamp(vec2i(ceil(hi)), first_cell + vec2i(1), last_cell + vec2i(1));
    var mean = vec3f(0.0);
    var mass = 0.0;
    var flags = 0u;
    for(var y = first.y; y < end.y; y++) {
        for(var x = first.x; x < end.x; x++) {
            let p = vec2i(x, y);
            // Actual boundary cells extend infinitely; acquired/tile edges do not.
            let a = select(vec2f(p), lo, p == first_cell);
            let b = select(vec2f(p + vec2i(1)), hi, p == last_cell);
            let overlap = max(vec2f(0.0), min(hi, b) - max(lo, a));
            let weight = overlap.x * overlap.y;
            if weight <= 0.0 {
                continue;
            }
            let sample = reconstruction_sample(p);
            let next = mass + weight;
            mean = mean * (1.0 - weight / next) + sample.rgb * (weight / next);
            mass = next;
            flags = flags | u32(sample.w);
        }
    }
    output[index] = vec4f(mean, f32(flags));
}
