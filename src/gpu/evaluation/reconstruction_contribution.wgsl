struct Contribution {
    mean: vec3f,
    mass: f32,
    flags: f32,
}

fn contribution(xy: vec2u, geometry: Geometry) -> Contribution {
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
            if weight <= 0.0 || !reconstruction_admitted(p, geometry) {
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
