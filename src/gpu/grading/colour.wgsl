const COLOUR_FOOT: f32 = 0.000030517578125;
const COLOUR_SCALE: f32 = 17.52;
const COLOUR_LOG_SCALE: f32 = 12.14393860341024;
const COLOUR_SLOPE: f32 = 2698.300861863559;
const COLOUR_CUT: f32 = 1.0 / COLOUR_SCALE;

fn colour_log1p(x: f32) -> f32 {
    // Cubic remainder below 0.001 is smaller than f32 rounding, without 1+x cancellation.
    if x < 0.001 {
        return x * fma(x, fma(x, 1.0 / 3.0, -0.5), 1.0);
    }
    return log(1.0 + x);
}

fn colour_expm1(x: f32) -> f32 {
    if x < 0.001 {
        return x * fma(x, fma(x, 1.0 / 6.0, 0.5), 1.0);
    }
    return exp(x) - 1.0;
}

// ACEScc relative to encoded zero; negative inputs follow the positive foot's tangent.
// The separate ACEScct LUT interchange encoding is unchanged.
fn colour_encode(x: f32) -> f32 {
    if x <= 0.0 {
        return COLOUR_SLOPE * x;
    }
    if x < COLOUR_FOOT {
        return colour_log1p(x / COLOUR_FOOT) / COLOUR_LOG_SCALE;
    }
    return (log2(x) + 16.0) / COLOUR_SCALE;
}

fn colour_decode(z: f32) -> f32 {
    if z <= 0.0 {
        return z / COLOUR_SLOPE;
    }
    if z < COLOUR_CUT {
        return COLOUR_FOOT * colour_expm1(z * COLOUR_LOG_SCALE);
    }
    return photographic_exp2(fma(z, COLOUR_SCALE, -16.0));
}

fn colour_tone(x: f32, da: f32, pivot: f32) -> f32 {
    let shift = -da * pivot;
    if x < -PHOTOGRAPHIC_LIMIT / COLOUR_SLOPE {
        // Stay in the negative linear continuation without overflowing its coordinate.
        return photographic_finite(fma(da, x, x) + shift / COLOUR_SLOPE);
    }
    let delta = colour_encode(x);
    let z = fma(da, delta, delta) + shift;
    if x <= 0.0 && z <= 0.0 {
        return photographic_finite(fma(da, x, x) + shift / COLOUR_SLOPE);
    }
    return colour_decode(z);
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

fn colour_oklab_roots(rgb: array<f32, 3>) -> vec3f {
    let cones = matrix_product(adjustment_layer_context.oklab_cones.columns, rgb);
    let lms = vec3f(cones[0], cones[1], cones[2]);
    return sign(lms) * pow(abs(lms), vec3f(1.0 / 3.0));
}

fn colour_preserve(
    original: array<f32, 3>,
    contrasted: array<f32, 3>,
    amount: f32,
) -> array<f32, 3> {
    let before = colour_oklab_roots(original);
    let after = colour_oklab_roots(contrasted);
    let difference = before - after;
    // Inverse OKLab's L column is (1,1,1): subtract only that component of the difference.
    let lightness = dot(adjustment_layer_context.oklab_lightness.xyz, difference);
    let roots = fma(vec3f(amount), difference - vec3f(lightness), after);
    var cubes: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        cubes[c] = photographic_finite(roots[c] * roots[c] * roots[c]);
    }
    return matrix_product(adjustment_layer_context.oklab_inverse.columns, cubes);
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
        for(var c = 0u; c < 3u; c++) {
            rgb[c] = colour_tone(rgb[c], da, pivot);
        }
        if colour != 0.0 && !(input[0] == input[1] && input[1] == input[2]) {
            rgb = colour_preserve(input, rgb, colour);
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
