// Stable f32 colour on filtered signed/HDR AP1; source geometry remains precise.
const V_LN2: f32 = 0.6931471805599453;
const V_TOE: f32 = 0.0078125;
const V_SLOPE: f32 = CCT_SLOPE.h + (CCT_SLOPE.l + CCT_SLOPE.t);
const V_OFFSET: f32 = CCT_OFFSET.h + (CCT_OFFSET.l + CCT_OFFSET.t);
const V_SCALE: f32 = CCT_SCALE.h + (CCT_SCALE.l + CCT_SCALE.t);
const V_LOG_SCALE: f32 = CCT_LOG_SCALE.h + (CCT_LOG_SCALE.l + CCT_LOG_SCALE.t);
const V_TOE_DELTA: f32 = V_SLOPE * V_TOE;
fn v_value(x: Triple) -> f32 { return x.h + (x.l + x.t); }
fn v_delta(row: u32, column: u32, base: f32) -> f32 {
    return (parameters.creative[row][column] - base) + (parameters.creative_low[row][column] + parameters.creative_tail[row][column]);
}
fn v_local_delta(stage: u32, row: u32, column: u32, base: f32) -> f32 {
    return (local_stages[stage].high[row][column] - base) + (local_stages[stage].low[row][column] + local_stages[stage].tail[row][column]);
}
fn v_local(stage: u32, row: u32, column: u32) -> f32 { return v_value(local_constant(stage, row, column)); }
fn v_matrix(inverse: bool, rgb: array<f32, 3>) -> array<f32, 3> {
    var result: array<f32, 3>;
    for (var c = 0u; c < 3u; c++) {
        var sum = 0.0;
        for (var j = 0u; j < 3u; j++) {
            var m: f32;
            if inverse { m = v_value(Triple(local_context.inverse.high[j][c], local_context.inverse.low[j][c], local_context.inverse.tail[j][c])); }
            else { m = v_value(Triple(local_context.cones.high[j][c], local_context.cones.low[j][c], local_context.cones.tail[j][c])); }
            sum = fma(m, rgb[j], sum);
        }
        result[c] = sum;
    }
    return result;
}
// Analytic expm1(x)/x limit, without epsilon division or a luminance clamp.
fn v_exprel(x: f32) -> f32 {
    if abs(x) <= 0.125 {
        var p = 1.0 / 5040.0;
        p = fma(x, p, 1.0 / 720.0);
        p = fma(x, p, 1.0 / 120.0);
        p = fma(x, p, 1.0 / 24.0);
        p = fma(x, p, 1.0 / 6.0);
        p = fma(x, p, 0.5);
        return fma(x, p, 1.0);
    }
    return (exp(x) - 1.0) / x;
}
fn v_log1p(x: f32) -> f32 {
    if abs(x) <= 0.125 {
        var p = -1.0 / 8.0;
        p = fma(x, p, 1.0 / 7.0);
        p = fma(x, p, -1.0 / 6.0);
        p = fma(x, p, 1.0 / 5.0);
        p = fma(x, p, -0.25);
        p = fma(x, p, 1.0 / 3.0);
        p = fma(x, p, -0.5);
        return x * fma(x, p, 1.0);
    }
    return log(1.0 + x);
}
// Black-relative CCT avoids losing small inputs to the encoding offset.
fn v_encode(x: f32) -> f32 {
    if x <= V_TOE { return V_SLOPE * x; }
    var stops = log2(x) + 7.0;
    if x < 2.0 * V_TOE { stops = v_log1p((x - V_TOE) / V_TOE) / V_LN2; }
    return fma(stops, 1.0 / V_SCALE, V_TOE_DELTA);
}
fn v_decode(z: f32) -> f32 {
    if z <= V_TOE_DELTA { return z / V_SLOPE; }
    // Include toe scaling in the exponent, avoiding discarded intermediate overflow.
    return exp2(fma(z - V_TOE_DELTA, V_SCALE, -7.0));
}
fn v_tone(x: f32, da: f32, pivot: f32) -> f32 {
    let shift = -da * pivot;
    let delta = v_encode(x);
    let z = fma(da, delta, delta) + shift;
    if x <= V_TOE && z <= V_TOE_DELTA { return fma(da, x, x) + shift / V_SLOPE; }
    return v_decode(z);
}
fn v_ratio(y: f32, da: f32, pivot: f32) -> f32 {
    let amount = 1.0 + da;
    let shift = -da * pivot;
    let black = v_decode(shift);
    var rate = amount * V_SLOPE;
    if y > V_TOE { rate = amount * (v_encode(y) / y); }
    let delta = rate * y;
    let to_cut = V_TOE_DELTA - shift;
    if delta == 0.0 {
        if to_cut >= 0.0 { return amount; }
        return black * V_LOG_SCALE * rate;
    }
    if to_cut >= 0.0 && delta <= to_cut { return rate / V_SLOPE; }
    if to_cut < 0.0 && delta > to_cut { return black * V_LOG_SCALE * rate * v_exprel(delta * V_LOG_SCALE); }
    // Integrate both sides of the continuous toe rather than subtracting decoded near-equals.
    let fraction = to_cut / delta;
    if to_cut >= 0.0 {
        let t = (delta - to_cut) * V_LOG_SCALE;
        if t > 0.5 { return fraction * rate / V_SLOPE + (v_decode(shift + delta) - V_TOE) / y; }
        return rate * (fraction / V_SLOPE + (1.0 - fraction) * V_TOE * V_LOG_SCALE * v_exprel(t));
    }
    return rate * (fraction * black * V_LOG_SCALE * v_exprel(to_cut * V_LOG_SCALE) + (1.0 - fraction) / V_SLOPE);
}
fn v_mix(a: f32, b: f32, t: f32) -> f32 {
    if t == 0.0 || a == b { return a; }
    if t == 1.0 { return b; }
    return fma(t, b - a, a);
}
fn v_luminance(rgb: array<f32, 3>) -> f32 {
    // Targeted compensated dot: signed RGB cancellation can become visible after later recovery.
    // Preserve coefficient and FMA product/sum residuals, returning ordinary f32 colour.
    let p = vec3f(parameters.creative[2].x * rgb[0], parameters.creative[2].y * rgb[1], parameters.creative[2].z * rgb[2]);
    let sum = p.x + p.y;
    let b = sum - p.x;
    let e = (p.x - (sum - b)) + (p.y - b);
    let total = sum + p.z;
    let d = total - sum;
    var correction = e + ((sum - (total - d)) + (p.z - d));
    for (var c = 0u; c < 3u; c++) {
        correction += fma(parameters.creative[2][c], rgb[c], -p[c]);
        correction = fma(parameters.creative_low[2][c], rgb[c], correction);
        correction = fma(parameters.creative_tail[2][c], rgb[c], correction);
    }
    return total + correction;
}
fn v_apply(input: array<f32, 3>, da: f32, pivot: f32, colour: f32, ds: f32, density: f32) -> array<f32, 3> {
    var rgb = input;
    if da != 0.0 {
        let black = v_decode(-da * pivot);
        var gain = 0.0;
        if colour != 0.0 { gain = v_ratio(v_luminance(rgb), da, pivot); }
        for (var c = 0u; c < 3u; c++) {
            if colour == 0.0 { rgb[c] = v_tone(rgb[c], da, pivot); }
            else if colour == 1.0 { rgb[c] = fma(gain, rgb[c], black); }
            else { rgb[c] = v_mix(v_tone(rgb[c], da, pivot), fma(gain, rgb[c], black), colour); }
        }
    }
    if ds == 0.0 || (rgb[0] == rgb[1] && rgb[1] == rgb[2]) { return rgb; }
    var grey = 0.0;
    if density != 1.0 { grey = v_luminance(rgb); }
    if density != 0.0 { grey = v_mix(grey, max(rgb[0], max(rgb[1], rgb[2])), density); }
    for (var c = 0u; c < 3u; c++) {
        if abs(ds) < 0.5 { rgb[c] = fma(ds, rgb[c] - grey, rgb[c]); }
        else { rgb[c] = fma(1.0 + ds, rgb[c] - grey, grey); }
    }
    return rgb;
}
fn v_global(rgb: array<f32, 3>) -> array<f32, 3> {
    return v_apply(rgb, v_delta(0u, 0u, 1.0), v_delta(0u, 1u, V_OFFSET), v_value(creative_constant(1u, 0u)), v_delta(1u, 1u, 1.0), v_value(creative_constant(1u, 2u)));
}
fn v_locals(input: array<f32, 3>, index: u32) -> array<f32, 3> {
    if local_context.count.x == 0u { return input; }
    let source = view_point(index);
    if !precise_le(number(f32(local_context.area.x)), source[0]) || !precise_le(number(f32(local_context.area.y)), source[1]) || precise_le(number(f32(local_context.area.x + local_context.area.z)), source[0]) || precise_le(number(f32(local_context.area.y + local_context.area.w)), source[1]) { return input; }
    var rgb = input;
    for (var stage = 0u; stage < local_context.count.x; stage++) {
        var coverage = local_coverage_point(stage, source);
        if local_stages[stage].flags.y != 0u { coverage = precise_sub(number(1.0), coverage); }
        let weight = v_value(precise_mul(coverage, local_constant(stage, 1u, 3u)));
        if weight == 0.0 { continue; }
        let ev = v_local(stage, 2u, 0u);
        if ev != 0.0 {
            let gain = exp(weight * ev);
            for (var c = 0u; c < 3u; c++) { rgb[c] *= gain; }
        }
        if local_stages[stage].flags.z != 0u {
            var cones = v_matrix(false, rgb);
            for (var c = 0u; c < 3u; c++) { cones[c] *= exp(weight * v_local(stage, 2u, c + 1u)); }
            rgb = v_matrix(true, cones);
        }
        let da = weight * v_local_delta(stage, 3u, 0u, 1.0);
        let ds = weight * v_local_delta(stage, 3u, 3u, 1.0);
        if da == 0.0 && ds == 0.0 { continue; }
        rgb = v_apply(rgb, da, v_local_delta(stage, 3u, 1u, V_OFFSET), v_local(stage, 3u, 2u), ds, v_local(stage, 4u, 0u));
    }
    return rgb;
}
@compute @workgroup_size(64)
fn view_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {
    let index = id.x + id.y * groups.x * 64u;
    if index >= arrayLength(&working_source) { return; }
    let input = working_source[index];
    if (u32(input.w) & 8u) == 0u { positives[index] = vec4f(0.0); return; }
    let gain = v_value(Triple(view.gain.x, view.gain.y, view.gain.z));
    let rgb = array<f32, 3>(input.x * gain, input.y * gain, input.z * gain);
    var post: array<f32, 3>;
    for (var c = 0u; c < 3u; c++) {
        var sum = 0.0;
        for (var j = 0u; j < 3u; j++) {
            sum = fma(v_value(Triple(parameters.post[j][c], parameters.post_low[j][c], parameters.post_tail[j][c])), rgb[j], sum);
        }
        post[c] = sum;
    }
    let result = v_locals(v_global(post), index);
    positives[index] = vec4f(result[0], result[1], result[2], input.w);
}
