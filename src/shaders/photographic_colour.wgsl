// Stable colour operators for physical-view and original-sample export grading.
const COLOUR_TOE: f32 = 0.0078125;
const COLOUR_SLOPE: f32 = CCT_SLOPE;
const COLOUR_OFFSET: f32 = CCT_OFFSET;
const COLOUR_SCALE: f32 = CCT_SCALE;
const COLOUR_LOG_SCALE: f32 = CCT_LOG_SCALE;
const COLOUR_TOE_DELTA: f32 = COLOUR_SLOPE * COLOUR_TOE;

fn colour_adjustment_layer_delta(stage: u32, row: u32, column: u32, base: f32) -> f32 {
    return adjustment_layer_value(stage, row, column) - base;
}

fn colour_adjustment_layer(stage: u32, row: u32, column: u32) -> f32 {
    return adjustment_layer_value(stage, row, column);
}

// Originals, repaired positives and filtered views share Roll balance and exposure.
// Neutral working-view acquisition omits exposure; its caller restores that gain here.
fn photographic_post(input: array<f32, 3>, gain: f32) -> array<f32, 3> {
    var scaled: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        scaled[c] = input[c] * gain;
    }
    return matrix_product(parameters.post, scaled);
}

fn colour_balance(stage: u32, rgb: array<f32, 3>, weight: f32) -> array<f32, 3> {
    let exposure = exp(weight * colour_adjustment_layer(stage, 2u, 0u));
    var exposed: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        exposed[c] = rgb[c] * exposure;
    }
    var cones = matrix_product(adjustment_layer_context.cones.columns, exposed);
    for(var c = 0u; c < 3u; c++) {
        cones[c] *= exp(weight * colour_adjustment_layer(stage, 2u, c + 1u));
    }
    return matrix_product(adjustment_layer_context.inverse.columns, cones);
}

fn colour_exprel(x: f32) -> f32 {
    if x == 0.0 {
        return 1.0;
    }
    return (exp(x) - 1.0) / x;
}

// Black-relative CCT avoids losing small inputs to the encoding offset.
fn colour_encode(x: f32) -> f32 {
    if x <= COLOUR_TOE {
        return COLOUR_SLOPE * x;
    }
    return fma(log2(x) + 7.0, 1.0 / COLOUR_SCALE, COLOUR_TOE_DELTA);
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

fn colour_luminance(rgb: array<f32, 3>) -> f32 {
    return dot(vec3f(0.2722287168, 0.6740817658, 0.0536895174), vec3f(rgb[0], rgb[1], rgb[2]));
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

fn colour_adjustment_layers(input: array<f32, 3>, source: vec2f) -> array<f32, 3> {
    if adjustment_layer_context.count.x == 0u {
        return input;
    }
    let area = adjustment_layer_context.area;
    let inside =
        f32(area.x) <= source.x && f32(area.y) <= source.y && source.x < f32(area.x + area.z)
        && source.y < f32(area.y + area.w);
    var rgb = input;
    for(var stage = 0u; stage < adjustment_layer_context.count.x; stage++) {
        if adjustment_layer_stages[stage].flags.x != 0u && !inside {
            continue;
        }
        var coverage = adjustment_layer_coverage_point32(stage, source);
        if adjustment_layer_stages[stage].flags.y != 0u {
            coverage = 1.0 - coverage;
        }
        let weight = coverage * adjustment_layer_value(stage, 1u, 3u);
        if weight == 0.0 {
            continue;
        }
        let ev = colour_adjustment_layer(stage, 2u, 0u);
        if adjustment_layer_stages[stage].flags.z != 0u {
            rgb = colour_balance(stage, rgb, weight);
        } else if ev != 0.0 {
            let gain = exp(weight * ev);
            for(var c = 0u; c < 3u; c++) {
                rgb[c] *= gain;
            }
        }
        let da = weight * colour_adjustment_layer_delta(stage, 3u, 0u, 1.0);
        let ds = weight * colour_adjustment_layer_delta(stage, 3u, 3u, 1.0);
        if da == 0.0 && ds == 0.0 {
            continue;
        }
        rgb = colour_apply(
                rgb,
                da,
                colour_adjustment_layer_delta(stage, 3u, 1u, COLOUR_OFFSET),
                colour_adjustment_layer(stage, 3u, 2u),
                ds,
                colour_adjustment_layer(stage, 4u, 0u),
            );
    }
    return rgb;
}
