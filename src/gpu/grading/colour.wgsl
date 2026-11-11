const CCT_SLOPE: f32 = 10.5402377416545;
const CCT_OFFSET: f32 = 0.0729055341958355;
const CCT_SCALE: f32 = 17.52;
const CCT_LOG_SCALE: f32 = 12.14393860341024;

// Stable colour operators for physical-view and original-sample export grading.
const COLOUR_TOE: f32 = 0.0078125;
const COLOUR_SLOPE: f32 = CCT_SLOPE;
const COLOUR_OFFSET: f32 = CCT_OFFSET;
const COLOUR_SCALE: f32 = CCT_SCALE;
const COLOUR_LOG_SCALE: f32 = CCT_LOG_SCALE;
const COLOUR_TOE_DELTA: f32 = COLOUR_SLOPE * COLOUR_TOE;

fn colour_exprel(x: f32) -> f32 {
    if x == 0.0 {
        return 1.0;
    }
    return (photographic_exp(x) - 1.0) / x;
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
    return photographic_exp2(fma(z - COLOUR_TOE_DELTA, COLOUR_SCALE, -7.0));
}

fn colour_tone(x: f32, da: f32, pivot: f32) -> f32 {
    let shift = -da * pivot;
    if x < -PHOTOGRAPHIC_LIMIT / COLOUR_SLOPE {
        // The negative toe is linear; do not overflow its encoded coordinate first.
        return photographic_finite(fma(da, x, x) + shift / COLOUR_SLOPE);
    }
    let delta = colour_encode(x);
    let z = fma(da, delta, delta) + shift;
    if x <= COLOUR_TOE && z <= COLOUR_TOE_DELTA {
        return photographic_finite(fma(da, x, x) + shift / COLOUR_SLOPE);
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
            gain = photographic_finite(colour_ratio(colour_luminance(rgb), da, pivot));
        }
        for(var c = 0u; c < 3u; c++) {
            if colour == 0.0 {
                rgb[c] = colour_tone(rgb[c], da, pivot);
            } else if colour == 1.0 {
                rgb[c] = photographic_finite(fma(gain, rgb[c], black));
            } else {
                rgb[c] = colour_mix(
                        colour_tone(rgb[c], da, pivot),
                        photographic_finite(fma(gain, rgb[c], black)),
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
            rgb[c] = photographic_finite(fma(ds, rgb[c] - grey, rgb[c]));
        } else {
            rgb[c] = photographic_finite(fma(1.0 + ds, rgb[c] - grey, grey));
        }
    }
    return rgb;
}
