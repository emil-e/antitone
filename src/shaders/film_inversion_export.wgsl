// Shared export response keeps compensated near-base subtraction, then one-word density/response
// only within the downstream sensitivity envelope. Other photographic stages stay shared.
fn response_expm1(x: f32) -> f32 {
    if abs(x) < 0.5 {
        var sum = EXP_FACTORIAL[17].h;
        for(var i = 17u; i > 1u; i--) {
            sum = fma(x, sum, EXP_FACTORIAL[i - 1u].h);
        }
        return x * sum;
    }
    return exp(x) - 1.0;
}

fn response_log_abs_expm1(x: f32) -> f32 {
    if x > 50.0 {
        return x;
    }
    if x > 0.0 {
        return x + log(-response_expm1(-x));
    }
    return log(-response_expm1(x));
}

fn response_log1p(z: f32) -> f32 {
    let r = z / (2.0 + z);
    let r2 = r * r;
    var sum = LOG_ODD[7].h;
    for(var i = 7u; i > 0u; i--) {
        sum = fma(r2, sum, LOG_ODD[i - 1u].h);
    }
    return 2.0 * r * sum;
}

// One f32 response for every input: the compensated inversion it used to fall back to was
// compiled into every pipeline and taken only for "sensitive" values.
fn invert_film(sample: Triple, c: u32) -> Triple {
    let relative = (parameters.dmin_floor[c] - sample.h) / sample.h;
    // Ratio density avoids subtracting large logs before low-gamma amplification.
    var l = log(parameters.dmin_floor[c] / sample.h);
    if abs(relative) < 0.5 {
        l = response_log1p(relative);
    }
    let beta = parameters.shape.x;
    let gamma = parameters.gamma_mode[c];
    let shoulder = parameters.shape.y;
    let a = beta * l / (gamma - shoulder * l / LN10_PRECISE.h);
    if a == 0.0 {
        return number(0.0);
    }
    let value = sign(a)
        * exp((response_log_abs_expm1(a) - parameters.anchor[c]) / beta + parameters.anchor.w);
    return number(value);
}
