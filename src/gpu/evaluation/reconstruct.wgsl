struct Geometry {
    sizes: vec4u,
    origins: vec4u,
    source_bounds: vec4u,
    sample_bounds: vec4u,
    start_dx: vec4f,
    dy: vec4f,
}
@group(0) @binding(21)
var<storage, read_write> output: array<vec4f>;
@group(0) @binding(22)
var<uniform> geometry: Geometry;
@group(0) @binding(23)
var<storage, read_write> metadata: array<f32>;
@group(0) @binding(24)
var<storage, read> means: array<vec4f>;

struct Contribution {
    mean: vec3f,
    mass: f32,
    flags: f32,
}

fn merged_flags(a: f32, b: f32) -> f32 {
    if a < 0.0 || b < 0.0 {
        return min(a, b);
    }
    return f32(u32(a) | u32(b));
}

fn weighted_mean(a: vec3f, b: vec3f, fraction: f32) -> vec3f {
    let difference = b - a;
    if all(abs(difference) <= vec3f(3.402823e38)) {
        if fraction <= 0.5 {
            return a + difference * fraction;
        }
        return b - difference * (1.0 - fraction);
    }
    // Opposite signed HDR endpoints can have an unrepresentable difference.
    return a * (1.0 - fraction) + b * fraction;
}

fn contribution(xy: vec2u) -> Contribution {
    let dx = geometry.start_dx.zw;
    let dy = geometry.dy.xy;
    let local_xy = vec2f(xy - geometry.origins.zw);
    let start = geometry.start_dx.xy + local_xy.x * dx + local_xy.y * dy;
    let points = array<vec2f, 4>(start, start + dx, start + dx + dy, start + dy);
    var low = points[0];
    var high = points[0];
    for(var i = 1u; i < 4u; i++) {
        low = min(low, points[i]);
        high = max(high, points[i]);
    }
    let anchor = vec2i(geometry.sample_bounds.xy);
    let first_cell = vec2i(geometry.source_bounds.xy) - anchor;
    let last_cell = vec2i(geometry.source_bounds.zw) - anchor - vec2i(1);
    var first = max(vec2i(floor(low)), vec2i(0));
    var end = min(vec2i(ceil(high)), vec2i(geometry.sample_bounds.zw - geometry.sample_bounds.xy));
    if geometry.dy.w != 0.0 {
        first = max(clamp(vec2i(floor(low)), first_cell, last_cell), vec2i(0));
        end = min(
                clamp(vec2i(ceil(high)), first_cell + vec2i(1), last_cell + vec2i(1)),
                vec2i(geometry.sample_bounds.zw - geometry.sample_bounds.xy),
            );
    }
    var mean = vec3f(0.0);
    var mass = 0.0;
    var flags = 0.0;
    for(var y = first.y; y < end.y; y++) {
        var row_mean = vec3f(0.0);
        var row_mass = 0.0;
        for(var x = first.x; x < end.x; x++) {
            let cell = vec2i(x, y);
            let p = cell + anchor;
            var weight = 0.0;
            if geometry.dy.z == 0.0 && geometry.dy.w != 0.0 {
                weight = extended_overlap(
                        start,
                        vec2f(cell),
                        dx,
                        dy,
                        vec2f(first_cell),
                        vec2f(last_cell + vec2i(1)),
                    );
            } else {
                weight = overlap(start, vec2f(cell), dx, dy);
            }
            if weight <= 0.0 || !reconstruction_admitted(p) {
                continue;
            }
            let sample = reconstruction_sample(p);
            let next = row_mass + weight;
            if row_mass == 0.0 {
                row_mean = sample.rgb;
            } else {
                row_mean = weighted_mean(row_mean, sample.rgb, weight / next);
            }
            row_mass = next;
            flags = merged_flags(flags, sample.w);
        }
        // Group rows before combining them: a long flat chain drifts when each row
        // repeats a small contrast boundary. Both levels retain convex HDR means.
        if row_mass > 0.0 {
            let next = mass + row_mass;
            if mass == 0.0 {
                mean = row_mean;
            } else {
                mean = weighted_mean(mean, row_mean, row_mass / next);
            }
            mass = next;
        }
    }
    return Contribution(mean, mass, flags);
}

fn covered_flags(flags: f32, mass: f32) -> f32 {
    let expected = select(
            0.0,
            abs(cross(geometry.start_dx.zw, geometry.dy.xy)),
            geometry.dy.z != 0.0,
        );
    return reconstruction_status(flags, mass, geometry.sizes.x, expected);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let value = contribution(xy);
    output[index] = vec4f(value.mean, covered_flags(value.flags, value.mass));
}

@compute @workgroup_size(64)
fn clear(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&output) {
        return;
    }
    output[index] = vec4f(0.0);
    metadata[index] = 0.0;
}

@compute @workgroup_size(64)
fn accumulate(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let value = contribution(xy);
    if value.mass <= 0.0 {
        return;
    }
    let local = xy - geometry.origins.xy;
    let destination = local.y * geometry.sizes.y + local.x;
    let previous = output[destination];
    if previous.w == 0.0 {
        output[destination] = vec4f(value.mean, value.mass);
        metadata[destination] = merged_flags(metadata[destination], value.flags);
        return;
    }
    let mass = previous.w + value.mass;
    let weight = value.mass / mass;
    output[destination] = vec4f(weighted_mean(previous.rgb, value.mean, weight), mass);
    metadata[destination] = merged_flags(metadata[destination], value.flags);
}

@compute @workgroup_size(64)
fn finish(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    let size = geometry.sizes.zw;
    if index >= size.x * size.y {
        return;
    }
    let xy = vec2u(index % size.x, index / size.x) + geometry.origins.zw;
    let local = xy - geometry.origins.xy;
    let destination = local.y * geometry.sizes.y + local.x;
    let value = means[index];
    output[destination] = vec4f(value.rgb, covered_flags(metadata[index], value.w));
}
