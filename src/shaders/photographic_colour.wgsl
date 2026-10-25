// Stable colour operators for physical-view and original-sample export grading.
const COLOUR_LN2: f32 = 0.6931471805599453;
const COLOUR_TOE: f32 = 0.0078125;
const COLOUR_SLOPE: f32 = CCT_SLOPE.h + (CCT_SLOPE.l + CCT_SLOPE.t);
const COLOUR_OFFSET: f32 = CCT_OFFSET.h + (CCT_OFFSET.l + CCT_OFFSET.t);
const COLOUR_SCALE: f32 = CCT_SCALE.h + (CCT_SCALE.l + CCT_SCALE.t);
const COLOUR_LOG_SCALE: f32 = CCT_LOG_SCALE.h + (CCT_LOG_SCALE.l + CCT_LOG_SCALE.t);
const COLOUR_TOE_DELTA: f32 = COLOUR_SLOPE * COLOUR_TOE;

fn colour_value(x: Triple) -> f32 {
    return x.h + (x.l + x.t);
}

fn colour_delta(row: u32, column: u32, base: f32) -> f32 {
    return (parameters.creative[row][column] - base)
        + (parameters.creative_low[row][column] + parameters.creative_tail[row][column]);
}

fn colour_local_delta(stage: u32, row: u32, column: u32, base: f32) -> f32 {
    return (local_stages[stage].high[row][column] - base)
        + (local_stages[stage].low[row][column] + local_stages[stage].tail[row][column]);
}

fn colour_local(stage: u32, row: u32, column: u32) -> f32 {
    return colour_value(local_constant(stage, row, column));
}

// Physical-view inputs are already FP32; one f32 evaluation serves every input.
fn colour_post(input: array<f32, 3>, gain: f32) -> array<f32, 3> {
    var rgb: array<f32, 3>;
    for (var c = 0u; c < 3u; c++) {
        rgb[c] = input[c] * gain;
    }
    var output: array<f32, 3>;
    for (var c = 0u; c < 3u; c++) {
        var value = 0.0;
        for (var j = 0u; j < 3u; j++) {
            let coefficient = colour_value(
                    Triple(
                        parameters.post[j][c],
                        parameters.post_low[j][c],
                        parameters.post_tail[j][c],
                    ),
                );
            value = fma(coefficient, rgb[j], value);
        }
        output[c] = value;
    }
    return output;
}

fn colour_wb_product(inverse: bool, rgb: array<vec2f, 3>) -> array<vec2f, 3> {
    var output: array<vec2f, 3>;
    for(var c = 0u; c < 3u; c++) {
        var value = 0.0;
        var magnitude = 0.0;
        for(var j = 0u; j < 3u; j++) {
            var coefficient: Triple;
            if inverse {
                coefficient = Triple(
                        local_context.inverse.high[j][c],
                        local_context.inverse.low[j][c],
                        local_context.inverse.tail[j][c],
                    );
            } else {
                coefficient = Triple(
                        local_context.cones.high[j][c],
                        local_context.cones.low[j][c],
                        local_context.cones.tail[j][c],
                    );
            }
            let m = colour_value(coefficient);
            value = fma(m, rgb[j].x, value);
            magnitude = fma(abs(m), rgb[j].y, magnitude);
        }
        output[c] = vec2f(value, magnitude);
    }
    return output;
}

fn colour_balance(stage: u32, rgb: array<f32, 3>, weight: f32) -> array<f32, 3> {
    let ev = colour_local(stage, 2u, 0u);
    var exposure = 1.0;
    if ev != 0.0 {
        exposure = exp(weight * ev);
    }
    var exposed: array<vec2f, 3>;
    for(var c = 0u; c < 3u; c++) {
        exposed[c] = vec2f(rgb[c] * exposure, abs(rgb[c]) * exposure);
    }
    var fast = colour_wb_product(false, exposed);
    for(var c = 0u; c < 3u; c++) {
        fast[c] *= exp(weight * colour_local(stage, 2u, c + 1u));
    }
    fast = colour_wb_product(true, fast);
    return array<f32, 3>(fast[0].x, fast[1].x, fast[2].x);
}

fn colour_exprel(x: f32) -> f32 {
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

fn colour_log1p(x: f32) -> f32 {
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
fn colour_encode(x: f32) -> f32 {
    if x <= COLOUR_TOE {
        return COLOUR_SLOPE * x;
    }
    var stops = log2(x) + 7.0;
    if x < 2.0 * COLOUR_TOE {
        stops = colour_log1p((x - COLOUR_TOE) / COLOUR_TOE) / COLOUR_LN2;
    }
    return fma(stops, 1.0 / COLOUR_SCALE, COLOUR_TOE_DELTA);
}

fn colour_decode(z: f32) -> f32 {
    if z <= COLOUR_TOE_DELTA {
        return z / COLOUR_SLOPE;
    }
    // Include toe scaling in the exponent, avoiding discarded intermediate overflow.
    return exp2(fma(z - COLOUR_TOE_DELTA, COLOUR_SCALE, -7.0));
}

fn colour_tone(x: f32, da: f32, pivot: f32) -> f32 {
    let shift = -da * pivot;
    let delta = colour_encode(x);
    let z = fma(da, delta, delta) + shift;
    if x <= COLOUR_TOE && z <= COLOUR_TOE_DELTA {
        return fma(da, x, x) + shift / COLOUR_SLOPE;
    }
    return colour_decode(z);
}

fn colour_ratio(y: f32, da: f32, pivot: f32) -> f32 {
    let amount = 1.0 + da;
    let shift = -da * pivot;
    let black = colour_decode(shift);
    var rate = amount * COLOUR_SLOPE;
    if y > COLOUR_TOE {
        rate = amount * (colour_encode(y) / y);
    }
    let delta = rate * y;
    let to_cut = COLOUR_TOE_DELTA - shift;
    if delta == 0.0 {
        if to_cut >= 0.0 {
            return amount;
        }
        return black * COLOUR_LOG_SCALE * rate;
    }
    if to_cut >= 0.0 && delta <= to_cut {
        return rate / COLOUR_SLOPE;
    }
    if to_cut < 0.0 && delta > to_cut {
        return black * COLOUR_LOG_SCALE * rate * colour_exprel(delta * COLOUR_LOG_SCALE);
    }
    // Integrate both sides of the continuous toe rather than subtracting decoded near-equals.
    let fraction = to_cut / delta;
    if to_cut >= 0.0 {
        let t = (delta - to_cut) * COLOUR_LOG_SCALE;
        if t > 0.5 {
            return fraction * rate / COLOUR_SLOPE + (colour_decode(shift + delta) - COLOUR_TOE) / y;
        }
        return rate
            * (fraction / COLOUR_SLOPE
                + (1.0 - fraction) * COLOUR_TOE * COLOUR_LOG_SCALE * colour_exprel(t));
    }
    return rate
        * (fraction * black * COLOUR_LOG_SCALE * colour_exprel(to_cut * COLOUR_LOG_SCALE)
            + (1.0 - fraction) / COLOUR_SLOPE);
}

fn colour_mix(a: f32, b: f32, t: f32) -> f32 {
    if t == 0.0 || a == b {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    return fma(t, b - a, a);
}

fn colour_dot(high: vec3f, low: vec3f, tail: vec3f, rgb: array<f32, 3>) -> f32 {
    // Targeted compensated dot: signed RGB cancellation can become visible after later recovery.
    // Preserve coefficient and FMA product/sum residuals, returning ordinary f32 colour.
    let p = vec3f(high.x * rgb[0], high.y * rgb[1], high.z * rgb[2]);
    let sum = p.x + p.y;
    let b = sum - p.x;
    let e = (p.x - (sum - b)) + (p.y - b);
    let total = sum + p.z;
    let d = total - sum;
    var correction = e + ((sum - (total - d)) + (p.z - d));
    for(var c = 0u; c < 3u; c++) {
        correction += fma(high[c], rgb[c], -p[c]);
        correction = fma(low[c], rgb[c], correction);
        correction = fma(tail[c], rgb[c], correction);
    }
    return total + correction;
}

fn colour_luminance(rgb: array<f32, 3>) -> f32 {
    return colour_dot(
            parameters.creative[2].xyz,
            parameters.creative_low[2].xyz,
            parameters.creative_tail[2].xyz,
            rgb,
        );
}

fn colour_apply(
    input: array<f32, 3>,
    da: f32,
    pivot: f32,
    colour: f32,
    ds: f32,
    density: f32,
) -> array<f32, 3> {
    var rgb = input;
    if da != 0.0 {
        let black = colour_decode(-da * pivot);
        var gain = 0.0;
        if colour != 0.0 {
            gain = colour_ratio(colour_luminance(rgb), da, pivot);
        }
        for(var c = 0u; c < 3u; c++) {
            if colour == 0.0 {
                rgb[c] = colour_tone(rgb[c], da, pivot);
            } else if colour == 1.0 {
                rgb[c] = fma(gain, rgb[c], black);
            } else {
                rgb[c] = colour_mix(
                        colour_tone(rgb[c], da, pivot),
                        fma(gain, rgb[c], black),
                        colour,
                    );
            }
        }
    }
    if ds == 0.0 || (rgb[0] == rgb[1] && rgb[1] == rgb[2]) {
        return rgb;
    }
    var grey = 0.0;
    if density != 1.0 {
        grey = colour_luminance(rgb);
    }
    if density != 0.0 {
        grey = colour_mix(grey, max(rgb[0], max(rgb[1], rgb[2])), density);
    }
    for(var c = 0u; c < 3u; c++) {
        if abs(ds) < 0.5 {
            rgb[c] = fma(ds, rgb[c] - grey, rgb[c]);
        } else {
            rgb[c] = fma(1.0 + ds, rgb[c] - grey, grey);
        }
    }
    return rgb;
}

fn colour_global(rgb: array<f32, 3>) -> array<f32, 3> {
    return colour_apply(
            rgb,
            colour_delta(0u, 0u, 1.0),
            colour_delta(0u, 1u, COLOUR_OFFSET),
            colour_value(creative_constant(1u, 0u)),
            colour_delta(1u, 1u, 1.0),
            colour_value(creative_constant(1u, 2u)),
        );
}

fn colour_locals(input: array<f32, 3>, source: vec2f) -> array<f32, 3> {
    if local_context.count.x == 0u {
        return input;
    }
    let area = local_context.area;
    if f32(area.x) > source.x
        || f32(area.y) > source.y
        || f32(area.x + area.z) <= source.x
        || f32(area.y + area.w) <= source.y {
        return input;
    }
    var rgb = input;
    for (var stage = 0u; stage < local_context.count.x; stage++) {
        var coverage = local_coverage_point32(stage, source);
        if local_stages[stage].flags.y != 0u {
            coverage = 1.0 - coverage;
        }
        let weight = coverage * local_value(stage, 1u, 3u);
        if weight == 0.0 {
            continue;
        }
        let ev = colour_local(stage, 2u, 0u);
        if local_stages[stage].flags.z != 0u {
            rgb = colour_balance(stage, rgb, weight);
        } else if ev != 0.0 {
            let gain = exp(weight * ev);
            for (var c = 0u; c < 3u; c++) {
                rgb[c] *= gain;
            }
        }
        let da = weight * colour_local_delta(stage, 3u, 0u, 1.0);
        let ds = weight * colour_local_delta(stage, 3u, 3u, 1.0);
        if da == 0.0 && ds == 0.0 {
            continue;
        }
        rgb = colour_apply(
                rgb,
                da,
                colour_local_delta(stage, 3u, 1u, COLOUR_OFFSET),
                colour_local(stage, 3u, 2u),
                ds,
                colour_local(stage, 4u, 0u),
            );
    }
    return rgb;
}

// Original pixel coordinates stay exact in f32 well beyond this application's image sizes.
fn colour_coordinate(value: u32) -> f32 {
    return f32(value);
}
