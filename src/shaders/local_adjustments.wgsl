struct LocalParameters {
    high: array<vec4f, 5>,
    low: array<vec4f, 5>,
    tail: array<vec4f, 5>,
    flags: vec4u,
}
struct LocalContext {
    cones: CameraMatrix,
    inverse: CameraMatrix,
    area: vec4u,
    count: vec4u,
}
@group(0) @binding(6) var<storage, read> local_stages: array<LocalParameters>;
@group(0) @binding(7) var<uniform> local_context: LocalContext;
fn local_constant(stage: u32, row: u32, column: u32) -> Triple {
    return Triple(local_stages[stage].high[row][column], local_stages[stage].low[row][column], local_stages[stage].tail[row][column]);
}
fn local_projection(delta: array<Triple, 2>, axis: array<Triple, 2>) -> Triple {
    return precise_add(precise_mul(delta[0], axis[0]), precise_mul(delta[1], axis[1]));
}
fn local_radius(x: Triple, y: Triple) -> Triple {
    // Extended accumulation and Newton corrections before rounding the source-space hypot.
    let square = add(multiply(x, x), multiply(y, y));
    if square.h == 0.0 { return number(0.0); }
    var root = number(sqrt(square.h));
    for (var i = 0u; i < 2u; i++) {
        let rem = add(square, precise_neg(multiply(root, root)));
        root = add(root, precise_div(rem, multiply(root, number(2.0))));
    }
    return round53(root);
}
fn local_smoothstep(t: Triple) -> Triple {
    if precise_le(t, number(0.0)) { return number(0.0); }
    if precise_le(number(1.0), t) { return number(1.0); }
    let near_start = precise_le(t, number(0.5));
    var u = t;
    if !near_start { u = precise_sub(number(1.0), t); }
    let value = precise_mul(precise_mul(u, u), precise_sub(number(3.0), precise_mul(number(2.0), u)));
    if near_start { return value; }
    return precise_sub(number(1.0), value);
}
fn local_coverage(stage: u32, source: vec2u) -> Triple {
    return local_coverage_point(stage, array<Triple, 2>(Triple(f32(source.x), 0.5, 0.0), Triple(f32(source.y), 0.5, 0.0)));
}
fn local_coverage_point(stage: u32, source: array<Triple, 2>) -> Triple {
    var delta: array<Triple, 2>;
    for (var c = 0u; c < 2u; c++) {
        delta[c] = precise_sub(source[c], local_constant(stage, 0u, c));
    }
    let x_axis = array<Triple, 2>(local_constant(stage, 0u, 2u), local_constant(stage, 0u, 3u));
    let radius_x = local_constant(stage, 4u, 1u);
    if local_stages[stage].flags.x == 1u {
        let normalized = array<Triple, 2>(precise_div(delta[0], radius_x), precise_div(delta[1], radius_x));
        return local_smoothstep(local_projection(normalized, x_axis));
    }
    let y_axis = array<Triple, 2>(local_constant(stage, 1u, 0u), local_constant(stage, 1u, 1u));
    let x = precise_div(local_projection(delta, x_axis), radius_x);
    let y = precise_div(local_projection(delta, y_axis), local_constant(stage, 4u, 2u));
    if precise_le(number(1.0), x) || precise_le(x, number(-1.0)) || precise_le(number(1.0), y) || precise_le(y, number(-1.0)) { return number(0.0); }
    let radius = local_radius(x, y);
    if precise_le(number(1.0), radius) { return number(0.0); }
    let feather = local_constant(stage, 1u, 2u);
    if precise_le(radius, precise_sub(number(1.0), feather)) { return number(1.0); }
    return local_smoothstep(precise_div(precise_sub(number(1.0), radius), feather));
}
fn local_matrix(inverse: bool, rgb: array<Triple, 3>) -> array<Triple, 3> {
    var output: array<Triple, 3>;
    for (var c = 0u; c < 3u; c++) {
        var sum = number(0.0);
        for (var j = 0u; j < 3u; j++) {
            var m: Triple;
            if inverse {
                m = Triple(local_context.inverse.high[j][c], local_context.inverse.low[j][c], local_context.inverse.tail[j][c]);
            } else {
                m = Triple(local_context.cones.high[j][c], local_context.cones.low[j][c], local_context.cones.tail[j][c]);
            }
            sum = precise_add(sum, precise_mul(m, rgb[j]));
        }
        output[c] = sum;
    }
    return output;
}
fn local_grade(input: array<Triple, 3>, index: u32) -> array<Triple, 3> {
    if local_context.count.x == 0u { return input; }
    let source = source_pixel(index);
    // Padded refinement samples are not original source pixels.
    if any(source < local_context.area.xy) || any(source >= local_context.area.xy + local_context.area.zw) { return input; }
    var rgb = input;
    for (var stage = 0u; stage < local_context.count.x; stage++) {
        var coverage = local_coverage(stage, source);
        if local_stages[stage].flags.y != 0u { coverage = precise_sub(number(1.0), coverage); }
        let weight = precise_mul(coverage, local_constant(stage, 1u, 3u));
        if precise_eq(weight, number(0.0)) { continue; }
        let ev = local_constant(stage, 2u, 0u);
        if !precise_eq(ev, number(0.0)) {
            let gain = precise_exp(precise_mul(weight, ev));
            for (var c = 0u; c < 3u; c++) { rgb[c] = precise_mul(rgb[c], gain); }
        }
        if local_stages[stage].flags.z != 0u {
            var cones = local_matrix(false, rgb);
            for (var c = 0u; c < 3u; c++) {
                cones[c] = precise_mul(cones[c], precise_exp(precise_mul(weight, local_constant(stage, 2u, c + 1u))));
            }
            rgb = local_matrix(true, cones);
        }
        let amount = precise_add(number(1.0), precise_mul(weight, precise_sub(local_constant(stage, 3u, 0u), number(1.0))));
        let saturation = precise_add(number(1.0), precise_mul(weight, precise_sub(local_constant(stage, 3u, 3u), number(1.0))));
        if precise_eq(amount, number(1.0)) && precise_eq(saturation, number(1.0)) { continue; }
        let colour = local_constant(stage, 3u, 2u);
        var qp = number(0.0);
        var q0 = number(0.0);
        var black = number(0.0);
        if !precise_eq(amount, number(1.0)) {
            qp = local_constant(stage, 3u, 1u);
            if !precise_eq(colour, number(0.0)) {
                q0 = precise_add(qp, precise_mul(amount, precise_sub(CCT_OFFSET, qp)));
                black = cct_decode(q0);
            }
        }
        rgb = apply_creative_grade(rgb, CreativeGrade(amount, qp, q0, black, colour, saturation, local_constant(stage, 4u, 0u)));
    }
    return rgb;
}
