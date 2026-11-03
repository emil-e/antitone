const STAGE_SOURCE: u32 = 0u;
const STAGE_NEGATIVE: u32 = 1u;
const STAGE_DIGITAL: u32 = 2u;

fn interpret_positive(scan: vec3f) -> ProcessedPixel {
    if parameters.stage == STAGE_DIGITAL {
        return ProcessedPixel(array<f32, 3>(scan.x, scan.y, scan.z), 0u);
    }
    var rgb: array<f32, 3>;
    var flags = 0u;
    for(var c = 0u; c < 3u; c++) {
        if scan[c] < parameters.dmin_floor.w {
            flags |= 1u << c;
        }
        rgb[c] = invert_film(max(scan[c], parameters.dmin_floor.w), c);
    }
    return ProcessedPixel(rgb, flags);
}

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
