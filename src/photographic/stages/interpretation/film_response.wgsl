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
        if scan[c] == parameters.dmax[c] && scan[c] > 0.0 {
            rgb[c] = photographic_exp(parameters.anchor.w);
        } else if scan[c] <= 0.0 {
            flags |= 1u << c;
            rgb[c] = parameters.tail[c].y + parameters.tail[c].z;
        } else {
            let ratio = scan[c] / parameters.dmin[c];
            var density: f32;
            if ratio > 0.0 && ratio <= 3.4028234663852886e+38 {
                density = -log(ratio);
            } else {
                // The ratio can underflow or overflow while its logarithm is finite.
                density = log(parameters.dmin[c]) - log(scan[c]);
            }
            if density > parameters.tail[c].x {
                flags |= 1u << c;
            }
            rgb[c] = invert_film(density, c);
        }
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

fn invert_film(density: f32, c: u32) -> f32 {
    let tail = parameters.tail[c];
    if density >= tail.x {
        return tail.y + tail.z * (1.0 - exp(tail.x - density));
    }
    let beta = parameters.shape.x;
    let gamma = parameters.gamma_mode[c];
    let h = parameters.shape.y / 2.302585092994046;
    var a: f32;
    if h * density > gamma * 0.5 {
        a = 4.0 * beta * density / gamma - beta / h;
    } else {
        a = beta * density / (gamma - h * density);
    }
    if a == 0.0 {
        return 0.0;
    }
    return sign(a) * photographic_exp(
            (response_log_abs_expm1(a) - parameters.anchor[c]) / beta + parameters.anchor.w,
        );
}
