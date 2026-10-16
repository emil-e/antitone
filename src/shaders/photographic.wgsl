const CCT_SLOPE: Triple = Triple(10.540237426757812, 3.1489668117501424e-7, 7.105427357601002e-15);
const CCT_OFFSET: Triple = Triple(0.072905533015728, 1.180107456200119e-9, 4.163336342344337e-17);
const CCT_CUT: Triple = Triple(0.155251145362854, -3.810343152110818e-9, 1.3877787807814457e-16);
const CCT_SCALE: Triple = Triple(
        17.520000457763672,
        -4.577636616431846e-7,
        -1.0658141036401503e-14,
    );
const CCT_ADD: Triple = Triple(9.720000267028809, -2.670288097306184e-7, 1.7763568394002505e-15);
const CCT_LOG_SCALE: Triple = Triple(
        12.143939018249512,
        -4.148392633851472e-7,
        -7.105427357601002e-15,
    );

fn precise_le(a: Triple, b: Triple) -> bool {
    return a.h < b.h || (a.h == b.h && (a.l < b.l || (a.l == b.l && a.t <= b.t)));
}

fn precise_eq(a: Triple, b: Triple) -> bool {
    return a.h == b.h && a.l == b.l && a.t == b.t;
}

fn creative_constant(row: u32, column: u32) -> Triple {
    return Triple(
            parameters.creative[row][column],
            parameters.creative_low[row][column],
            parameters.creative_tail[row][column],
        );
}

fn cct_encode(x: Triple) -> Triple {
    if precise_le(x, number(0.0078125)) {
        return precise_add(precise_mul(CCT_SLOPE, x), CCT_OFFSET);
    }
    return precise_div(precise_add(precise_div(precise_log(x), LN2_PRECISE), CCT_ADD), CCT_SCALE);
}

fn cct_decode(q: Triple) -> Triple {
    if precise_le(q, CCT_CUT) {
        return precise_div(precise_sub(q, CCT_OFFSET), CCT_SLOPE);
    }
    return precise_exp(precise_mul(precise_sub(precise_mul(q, CCT_SCALE), CCT_ADD), LN2_PRECISE));
}

struct CreativeGrade {
    amount: Triple,
    qp: Triple,
    q0: Triple,
    black: Triple,
    colour: Triple,
    saturation: Triple,
    density: Triple,
}

fn grade_tone(x: Triple, grade: CreativeGrade) -> Triple {
    return cct_decode(
            precise_add(grade.qp, precise_mul(grade.amount, precise_sub(cct_encode(x), grade.qp))),
        );
}

fn grade_y(rgb: array<Triple, 3>) -> Triple {
    var sum = number(0.0);
    for(var c = 0u; c < 3u; c++) {
        sum = precise_add(sum, precise_mul(creative_constant(2u, c), rgb[c]));
    }
    return sum;
}

fn ratio_gain(y: Triple, grade: CreativeGrade) -> Triple {
    let amount = grade.amount;
    let q0 = grade.q0;
    let black = grade.black;
    var delta = precise_mul(CCT_SLOPE, y);
    if !precise_le(y, number(0.0078125)) {
        delta = precise_sub(cct_encode(y), CCT_OFFSET);
    }
    delta = precise_mul(amount, delta);
    let qy = precise_add(q0, delta);
    if precise_eq(y, number(0.0)) {
        if precise_le(q0, CCT_CUT) {
            return amount;
        }
        return precise_mul(precise_mul(precise_mul(black, CCT_LOG_SCALE), amount), CCT_SLOPE);
    }

    if precise_le(q0, CCT_CUT) && precise_le(qy, CCT_CUT) {
        if precise_le(y, number(0.0078125)) {
            return amount;
        }
        return precise_div(delta, precise_mul(CCT_SLOPE, y));
    }

    if !precise_le(q0, CCT_CUT) && !precise_le(qy, CCT_CUT) {
        return precise_div(precise_mul(black, precise_expm1(precise_mul(delta, CCT_LOG_SCALE))), y);
    }

    // Integrate the continuous toe instead of subtracting near-equal decodes.
    let to_cut = precise_sub(CCT_CUT, q0);
    if precise_le(q0, CCT_CUT) {
        let linear = precise_div(to_cut, CCT_SLOPE);
        let curved = precise_mul(
                cct_decode(CCT_CUT),
                precise_expm1(precise_mul(precise_sub(delta, to_cut), CCT_LOG_SCALE)),
            );
        return precise_div(precise_add(linear, curved), y);
    }
    let curved = precise_mul(black, precise_expm1(precise_mul(to_cut, CCT_LOG_SCALE)));
    let linear = precise_div(precise_sub(delta, to_cut), CCT_SLOPE);
    return precise_div(precise_add(curved, linear), y);
}

fn grade_mix(a: Triple, b: Triple, t: Triple) -> Triple {
    if precise_eq(t, number(0.0)) || precise_eq(a, b) {
        return a;
    }
    if precise_eq(t, number(1.0)) {
        return b;
    }
    return precise_add(precise_mul(precise_sub(number(1.0), t), a), precise_mul(t, b));
}

fn global_creative_grade() -> CreativeGrade {
    return CreativeGrade(
            creative_constant(0u, 0u),
            creative_constant(0u, 1u),
            creative_constant(0u, 2u),
            creative_constant(0u, 3u),
            creative_constant(1u, 0u),
            creative_constant(1u, 1u),
            creative_constant(1u, 2u),
        );
}

fn photographic_grade(input: array<Triple, 3>) -> array<Triple, 3> {
    return apply_creative_grade(input, global_creative_grade());
}

fn apply_creative_grade(input: array<Triple, 3>, grade: CreativeGrade) -> array<Triple, 3> {
    var rgb = input;
    if !precise_eq(grade.amount, number(1.0)) {
        let colour = grade.colour;
        var gain = number(0.0);
        if !precise_eq(colour, number(0.0)) {
            gain = ratio_gain(grade_y(rgb), grade);
        }
        for(var c = 0u; c < 3u; c++) {
            if precise_eq(colour, number(0.0)) {
                rgb[c] = grade_tone(rgb[c], grade);
            } else if precise_eq(colour, number(1.0)) {
                rgb[c] = precise_add(grade.black, precise_mul(gain, rgb[c]));
            } else {
                let coupled = grade_tone(rgb[c], grade);
                let proxy = precise_add(grade.black, precise_mul(gain, rgb[c]));
                rgb[c] = grade_mix(coupled, proxy, colour);
            }
        }
    }
    let saturation = grade.saturation;
    if precise_eq(saturation, number(1.0))
        || (precise_eq(rgb[0], rgb[1]) && precise_eq(rgb[1], rgb[2])) {
        return rgb;
    }
    let density = grade.density;
    var grey = number(0.0);
    if !precise_eq(density, number(1.0)) {
        grey = grade_y(rgb);
    }
    var maximum = rgb[0];
    if !precise_eq(density, number(0.0)) {
        for(var c = 1u; c < 3u; c++) {
            if precise_le(maximum, rgb[c]) {
                maximum = rgb[c];
            }
        }
    }
    for(var c = 0u; c < 3u; c++) {
        if precise_eq(density, number(0.0)) {
            rgb[c] = precise_add(grey, precise_mul(saturation, precise_sub(rgb[c], grey)));
        } else if precise_eq(density, number(1.0)) {
            rgb[c] = precise_add(maximum, precise_mul(saturation, precise_sub(rgb[c], maximum)));
        } else {
            let constant_y = precise_add(grey, precise_mul(saturation, precise_sub(rgb[c], grey)));
            let hsv = precise_add(maximum, precise_mul(saturation, precise_sub(rgb[c], maximum)));
            rgb[c] = grade_mix(constant_y, hsv, density);
        }
    }
    return rgb;
}
