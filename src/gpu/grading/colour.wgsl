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

fn colour_oklab_roots(rgb: array<f32, 3>) -> vec3f {
    let cones = matrix_product(adjustment_layer_context.oklab_cones.columns, rgb);
    let lms = vec3f(cones[0], cones[1], cones[2]);
    return sign(lms) * pow(abs(lms), vec3f(1.0 / 3.0));
}

fn colour_preserve(original: array<f32, 3>, after: vec3f, amount: f32) -> vec3f {
    let difference = colour_oklab_roots(original) - after;
    // Inverse OKLab's L column is (1,1,1): subtract only that component of the difference.
    let lightness = dot(adjustment_layer_context.oklab_lightness.xyz, difference);
    return fma(vec3f(amount), difference - vec3f(lightness), after);
}

fn colour_apply(
    input: array<f32, 3>,
    da: f32,
    pivot: f32,
    colour: f32,
    ds: f32,
    vibrance: f32,
) -> array<f32, 3> {
    var rgb = input;
    if da != 0.0 {
        for(var c = 0u; c < 3u; c++) {
            rgb[c] = colour_tone(rgb[c], da, pivot);
        }
    }
    let preserve = da != 0.0 && colour != 0.0;
    if (!preserve && ds == 0.0 && vibrance == 0.0)
        || (input[0] == input[1] && input[1] == input[2]) {
        return rgb;
    }
    // Keep colour preservation and chroma adjustment in the same roots, without
    // an ill-conditioned RGB round trip near zero cone responses.
    var roots = colour_oklab_roots(rgb);
    if preserve {
        roots = colour_preserve(input, roots, colour);
    }
    var adjusted = roots;
    if ds != 0.0 || vibrance != 0.0 {
        let lightness = dot(adjustment_layer_context.oklab_lightness.xyz, roots);
        var gain = 1.0 + ds;
        if vibrance != 0.0 {
            let a = dot(vec3f(1.977998539071675, -2.4285922502018127, 0.4505937111301374), roots);
            let b = dot(
                    vec3f(0.025904030547901084, 0.7827717270900503, -0.8086757576379514),
                    roots,
                );
            // Half response at C/|L| = 1/4, independent of exposure and smooth at neutral.
            let muted = (0.25 * lightness) * (0.25 * lightness);
            let total = muted + a * a + b * b;
            if total != 0.0 {
                gain *= 1.0 + vibrance * muted / total;
            }
        }
        adjusted = fma(vec3f(gain), roots - vec3f(lightness), vec3f(lightness));
    }
    var cubes: array<f32, 3>;
    for(var c = 0u; c < 3u; c++) {
        cubes[c] = photographic_finite(adjusted[c] * adjusted[c] * adjusted[c]);
    }
    return matrix_product(adjustment_layer_context.oklab_inverse.columns, cubes);
}
