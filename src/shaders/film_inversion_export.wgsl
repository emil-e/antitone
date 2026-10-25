// Ordinary f32 evaluation, including the continuous continuation beyond film base.
fn response_log_abs_expm1(x: f32) -> f32 {
    // log(exp(x) - 1) rounds to x here without risking exponential overflow.
    if x > 50.0 {
        return x;
    }
    return log(abs(exp(x) - 1.0));
}

fn invert_film(sample: f32, c: u32) -> f32 {
    let density = log(parameters.dmin_floor[c] / sample);
    let beta = parameters.shape.x;
    let a = beta * density
        / (parameters.gamma_mode[c] - parameters.shape.y * density / 2.302585092994046);
    if a == 0.0 {
        return 0.0;
    }
    return sign(a)
        * exp((response_log_abs_expm1(a) - parameters.anchor[c]) / beta + parameters.anchor.w);
}
