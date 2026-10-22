// Three-part temporaries retain f64 constants and rounding through response and AP1 cancellation.
const LN2_PRECISE: Triple = Triple(
        0.6931471824645996,
        -1.9046542121259336e-9,
        -1.1102230246251565e-16,
    );
const LN10_PRECISE: Triple = Triple(
        2.3025851249694824,
        -3.1975435632602967e-8,
        -8.881784197001252e-16,
    );
// Rounded-f64 coefficients shared by ordinary and full-series EXR math.
// Horner evaluation avoids a compensated division in every term.
const LOG_ODD: array<Triple, 15> = array<Triple, 15>(
        Triple(1.0, 0.0, 0.0),
        Triple(0.3333333432674408, -9.934107758624577e-9, 2.7755575615628914e-16),
        Triple(0.20000000298023224, -2.9802322831784522e-9, 5.551115123125783e-17),
        Triple(0.1428571492433548, -6.38621200366174e-9, 5.551115123125783e-17),
        Triple(0.1111111119389534, -8.278422947149977e-10, 0.0),
        Triple(0.09090909361839294, -2.709302115988521e-9, 8.326672684688674e-17),
        Triple(0.07692307978868484, -2.8656079731348427e-9, 5.551115123125783e-17),
        Triple(0.06666667014360428, -3.47693762670076e-9, 1.3877787807814457e-17),
        Triple(0.05882352963089943, -2.1913472425527658e-10, 0.0),
        Triple(0.05263157933950424, -3.9213582381236733e-10, 0.0),
        Triple(0.0476190485060215, -8.869738832295582e-10, 0.0),
        Triple(0.043478261679410934, -8.098457460192776e-10, 2.7755575615628914e-17),
        Triple(0.03999999910593033, 8.940696516468449e-10, 2.0816681711721685e-17),
        Triple(0.03703703731298447, -2.759474315716659e-10, 0.0),
        Triple(0.03448275849223137, 1.284582856753147e-10, 0.0),
    );
const EXP_FACTORIAL: array<Triple, 18> = array<Triple, 18>(
        Triple(1.0, 0.0, 0.0),
        Triple(1.0, 0.0, 0.0),
        Triple(0.5, 0.0, 0.0),
        Triple(0.1666666716337204, -4.967053879312289e-9, 1.3877787807814457e-16),
        Triple(0.0416666679084301, -1.2417634698280722e-9, 3.469446951953614e-17),
        Triple(0.008333333767950535, -4.34617203337595e-10, 1.734723475976807e-18),
        Triple(0.0013888889225199819, -3.3631094437103215e-11, 1.5178830414797062e-18),
        Triple(0.00019841270113829523, -2.725596874933456e-12, 5.421010862427522e-20),
        Triple(2.4801587642286904e-5, -3.40699609366682e-13, 6.776263578034403e-21),
        Triple(2.7557318844628753e-6, 3.793571224297229e-14, 1.6940658945086007e-21),
        Triple(2.755731998149713e-7, -7.575112209051195e-15, -2.117582368135751e-22),
        Triple(2.5052107943679403e-8, 4.4176230446483665e-16, 1.3234889800848443e-23),
        Triple(2.0876755879584152e-9, 1.1082839809204342e-16, -3.308722450212111e-24),
        Triple(1.6059044372074283e-10, -5.352526511562726e-18, -1.809457589959748e-25),
        Triple(1.147074536050896e-11, 2.372207689231238e-19, -4.8467614016778965e-27),
        Triple(7.647163609812713e-13, 1.2200710471178288e-20, -1.0097419586828951e-28),
        Triple(4.7794772561329454e-14, 7.62544404448643e-22, -6.310887241768095e-30),
        Triple(2.8114573589663704e-15, -1.0462084739763658e-22, -2.3665827156630354e-30),
    );

fn number(a: f32) -> Triple {
    return Triple(a, 0.0, 0.0);
}

fn precise_neg(a: Triple) -> Triple {
    return Triple(-a.h, -a.l, -a.t);
}

fn precise_add(a: Triple, b: Triple) -> Triple {
    return round53(add(a, b));
}

fn precise_sub(a: Triple, b: Triple) -> Triple {
    return precise_add(a, precise_neg(b));
}

fn precise_mul(a: Triple, b: Triple) -> Triple {
    return round53(multiply(a, b));
}

fn precise_div(a: Triple, b: Triple) -> Triple {
    let h = a.h / b.h;
    let rem = add(a, precise_neg(multiply(number(h), b)));
    let l = rem.h / b.h;
    let tail = add(rem, precise_neg(multiply(number(l), b)));
    return round53(add(number(h), add(number(l), number(tail.h / b.h))));
}

fn precise_log(x: Triple) -> Triple {
    let bits = bitcast<u32>(x.h);
    let exponent = (bits >> 23u) & 255u;
    var n = i32(exponent) - 127;
    let scale = bitcast<f32>((254u - exponent) << 23u);
    var m = Triple(bitcast<f32>((bits & 0x7fffffu) | (127u << 23u)), x.l * scale, x.t * scale);
    if m.h > 1.4142135623730951 {
        m = precise_mul(m, number(0.5));
        n += 1;
    }
    return precise_add(
            precise_log1p(precise_sub(m, number(1.0))),
            precise_mul(number(f32(n)), LN2_PRECISE),
        );
}

fn precise_log_ratio(base: Triple, sample: Triple) -> Triple {
    let delta = precise_sub(base, sample);
    let z = precise_div(delta, sample);
    if abs(z.h) < 0.5 {
        return precise_log1p(z);
    }
    return precise_sub(precise_log(base), precise_log(sample));
}

fn precise_log_abs_expm1(x: Triple) -> Triple {
    if x.h > 50.0 {
        return x;
    }
    if x.h > 0.0 {
        return precise_add(x, precise_log(precise_neg(precise_expm1(precise_neg(x)))));
    }
    return precise_log(precise_neg(precise_expm1(x)));
}

fn precise_invert_film(sample: Triple, c: u32) -> Triple {
    let base = Triple(parameters.dmin_floor[c], parameters.dmin_low[c], parameters.dmin_tail[c]);
    let l = precise_log_ratio(base, sample);
    let beta = Triple(parameters.shape.x, parameters.shape.z, parameters.shape_tail.x);
    let k = Triple(parameters.shape.y, parameters.shape.w, parameters.shape_tail.y);
    let gamma = Triple(parameters.gamma_mode[c], parameters.gamma_low[c], parameters.gamma_tail[c]);
    let density = precise_div(l, LN10_PRECISE);
    let a = precise_div(precise_mul(beta, l), precise_sub(gamma, precise_mul(k, density)));
    if precise_eq(a, number(0.0)) {
        return number(0.0);
    }
    let anchor = Triple(parameters.anchor[c], parameters.anchor_low[c], parameters.anchor_tail[c]);
    let anchored = precise_div(precise_sub(precise_log_abs_expm1(a), anchor), beta);
    let gain = Triple(parameters.anchor.w, parameters.anchor_low.w, parameters.anchor_tail.w);
    return precise_mul(number(sign(a.h)), precise_exp(precise_add(anchored, gain)));
}

struct ProcessedPixel {
    rgb: array<Triple, 3>,
    flags: u32,
}

// Input stage: source sample → scene-referred native-camera positive. The switch is
// confined here — a twin of the CPU `Interpretation::interpret`. Everything downstream
// (`camera_positive_to_ap1`, `photographic_post`) is stage-blind.
const STAGE_DIGITAL: u32 = 2u;

fn interpret_positive(scan: ConvertedScan) -> ProcessedPixel {
    switch u32(parameters.gamma_mode.w) {
        case STAGE_DIGITAL: {
            // Already a scene-referred positive: pass through, no polarity flip or
            // density flooring. The exposure gain rides in `post`, not the response.
            return ProcessedPixel(
                array<Triple, 3>(
                    Triple(scan.high[0], scan.low[0], scan.tail[0]),
                    Triple(scan.high[1], scan.low[1], scan.tail[1]),
                    Triple(scan.high[2], scan.low[2], scan.tail[2]),
                ),
                0u,
            );
        }
        default: {
            return negative_positive(scan);
        }
    }
}

// Negative film: invert between the measured Dmin/Dmax anchors.
fn negative_positive(scan: ConvertedScan) -> ProcessedPixel {
    var q: array<Triple, 3>;
    var flags = 0u;
    let floor = Triple(parameters.dmin_floor.w, parameters.dmin_low.w, parameters.dmin_tail.w);
    for(var c = 0u; c < 3u; c++) {
        var s = Triple(scan.high[c], scan.low[c], scan.tail[c]);
        if !precise_le(floor, s) {
            flags |= 1u << c;
            s = floor;
        }
        q[c] = invert_film(s, c);
    }
    return ProcessedPixel(q, flags);
}

// Shared, stage-blind: interpret → camera→AP1 → global grade.
fn process_film(scan: ConvertedScan) -> ProcessedPixel {
    let positive = interpret_positive(scan);
    return ProcessedPixel(photographic_post(camera_positive_to_ap1(positive.rgb)), positive.flags);
}

fn process_camera_source(scan: ConvertedScan) -> vec4f {
    let working = camera_source_to_ap1(scan);
    let q = array<Triple, 3>(
            Triple(working.high.x, working.low.x, working.tail.x),
            Triple(working.high.y, working.low.y, working.tail.y),
            Triple(working.high.z, working.low.z, working.tail.z),
        );
    return finish_pixel(ProcessedPixel(photographic_post(q), 0u));
}

fn camera_positive_to_ap1(q: array<Triple, 3>) -> array<Triple, 3> {
    var working: array<Triple, 3>;
    for(var c = 0u; c < 3u; c++) {
        var sum = number(0.0);
        for(var j = 0u; j < 3u; j++) {
            let m = Triple(
                    camera_matrix.high[j][c],
                    camera_matrix.low[j][c],
                    camera_matrix.tail[j][c],
                );
            sum = precise_add(sum, precise_mul(m, q[j]));
        }
        working[c] = sum;
    }
    return working;
}

fn photographic_post(working: array<Triple, 3>) -> array<Triple, 3> {
    var rgb: array<Triple, 3>;
    for(var c = 0u; c < 3u; c++) {
        let x = precise_mul(
                Triple(
                    parameters.post[0][c],
                    parameters.post_low[0][c],
                    parameters.post_tail[0][c],
                ),
                working[0],
            );
        let y = precise_mul(
                Triple(
                    parameters.post[1][c],
                    parameters.post_low[1][c],
                    parameters.post_tail[1][c],
                ),
                working[1],
            );
        let z = precise_mul(
                Triple(
                    parameters.post[2][c],
                    parameters.post_low[2][c],
                    parameters.post_tail[2][c],
                ),
                working[2],
            );
        rgb[c] = precise_add(precise_add(x, y), z);
    }
    // Source inspection shares the precise matrix stage, never the creative grade.
    if parameters.gamma_mode.w > 0.5 {
        rgb = photographic_grade(rgb);
    }
    return rgb;
}

fn finish_pixel(pixel: ProcessedPixel) -> vec4f {
    var result = vec3f(0.0);
    for(var c = 0u; c < 3u; c++) {
        result[c] = fma(1.0, pixel.rgb[c].h, fma(1.0, pixel.rgb[c].l, pixel.rgb[c].t));
    }
    return vec4f(result, f32(pixel.flags));
}
