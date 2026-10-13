// Keep AP1 cancellation and near-zero-Y divided differences in temporary pairs.
const CCT_SLOPE: Pair = Pair(10.540237426757812, 3.1489668117501424e-7);
const CCT_OFFSET: Pair = Pair(0.072905533015728, 1.180107456200119e-9);
const CCT_CUT: Pair = Pair(0.155251145362854, -3.810343152110818e-9);
const CCT_SCALE: Pair = Pair(17.520000457763672, -4.577636616431846e-7);
const CCT_ADD: Pair = Pair(9.720000267028809, -2.670288097306184e-7);
const CCT_LOG_SCALE: Pair = Pair(12.143939018249512, -4.148392633851472e-7);
fn pair_le(a: Pair, b: Pair) -> bool {
    return a.h < b.h || (a.h == b.h && a.l <= b.l);
}
fn pair_eq(a: Pair, b: Pair) -> bool { return a.h == b.h && a.l == b.l; }
fn creative_constant(row: u32, column: u32) -> Pair {
    return Pair(parameters.creative[row][column], parameters.creative_low[row][column]);
}
fn cct_encode(x: Pair) -> Pair {
    if pair_le(x, pair(0.0078125)) { return pair_add(pair_mul(CCT_SLOPE, x), CCT_OFFSET); }
    return pair_div(pair_add(pair_div(pair_log(x), LN2_PAIR), CCT_ADD), CCT_SCALE);
}
fn cct_decode(q: Pair) -> Pair {
    if pair_le(q, CCT_CUT) { return pair_div(pair_sub(q, CCT_OFFSET), CCT_SLOPE); }
    return pair_exp(pair_mul(pair_sub(pair_mul(q, CCT_SCALE), CCT_ADD), LN2_PAIR));
}
fn grade_tone(x: Pair) -> Pair {
    let qp = creative_constant(0u, 1u);
    return cct_decode(pair_add(qp, pair_mul(creative_constant(0u, 0u), pair_sub(cct_encode(x), qp))));
}
fn grade_y(rgb: array<Pair, 3>) -> Pair {
    var sum = pair(0.0);
    for (var c = 0u; c < 3u; c++) {
        sum = pair_add(sum, pair_mul(creative_constant(2u, c), rgb[c]));
    }
    return sum;
}
fn ratio_gain(y: Pair) -> Pair {
    let amount = creative_constant(0u, 0u);
    let q0 = creative_constant(0u, 2u);
    let black = creative_constant(0u, 3u);
    var delta = pair_mul(CCT_SLOPE, y);
    if !pair_le(y, pair(0.0078125)) { delta = pair_sub(cct_encode(y), CCT_OFFSET); }
    delta = pair_mul(amount, delta);
    let qy = pair_add(q0, delta);
    if pair_eq(y, pair(0.0)) {
        if pair_le(q0, CCT_CUT) { return amount; }
        return pair_mul(pair_mul(pair_mul(black, CCT_LOG_SCALE), amount), CCT_SLOPE);
    }
    if pair_le(q0, CCT_CUT) && pair_le(qy, CCT_CUT) {
        if pair_le(y, pair(0.0078125)) { return amount; }
        return pair_div(delta, pair_mul(CCT_SLOPE, y));
    }
    if !pair_le(q0, CCT_CUT) && !pair_le(qy, CCT_CUT) {
        return pair_div(pair_mul(black, pair_expm1(pair_mul(delta, CCT_LOG_SCALE))), y);
    }
    // Integrate the continuous toe instead of subtracting near-equal decodes.
    let to_cut = pair_sub(CCT_CUT, q0);
    if pair_le(q0, CCT_CUT) {
        let linear = pair_div(to_cut, CCT_SLOPE);
        let curved = pair_mul(cct_decode(CCT_CUT), pair_expm1(pair_mul(pair_sub(delta, to_cut), CCT_LOG_SCALE)));
        return pair_div(pair_add(linear, curved), y);
    }
    let curved = pair_mul(black, pair_expm1(pair_mul(to_cut, CCT_LOG_SCALE)));
    let linear = pair_div(pair_sub(delta, to_cut), CCT_SLOPE);
    return pair_div(pair_add(curved, linear), y);
}
fn grade_mix(a: Pair, b: Pair, t: Pair) -> Pair {
    if pair_eq(t, pair(0.0)) || pair_eq(a, b) { return a; }
    if pair_eq(t, pair(1.0)) { return b; }
    return pair_add(pair_mul(pair_sub(pair(1.0), t), a), pair_mul(t, b));
}
fn photographic_grade(input: array<Pair, 3>) -> array<Pair, 3> {
    var rgb = input;
    if !pair_eq(creative_constant(0u, 0u), pair(1.0)) {
        let gain = ratio_gain(grade_y(rgb));
        let black = creative_constant(0u, 3u);
        for (var c = 0u; c < 3u; c++) {
            let coupled = grade_tone(rgb[c]);
            let proxy = pair_add(black, pair_mul(gain, rgb[c]));
            rgb[c] = grade_mix(coupled, proxy, creative_constant(1u, 0u));
        }
    }
    let saturation = creative_constant(1u, 1u);
    if pair_eq(saturation, pair(1.0)) || (pair_eq(rgb[0], rgb[1]) && pair_eq(rgb[1], rgb[2])) { return rgb; }
    let grey = grade_y(rgb);
    var maximum = rgb[0];
    for (var c = 1u; c < 3u; c++) {
        if pair_le(maximum, rgb[c]) { maximum = rgb[c]; }
    }
    for (var c = 0u; c < 3u; c++) {
        let constant_y = pair_add(grey, pair_mul(saturation, pair_sub(rgb[c], grey)));
        let hsv = pair_add(maximum, pair_mul(saturation, pair_sub(rgb[c], maximum)));
        rgb[c] = grade_mix(constant_y, hsv, creative_constant(1u, 2u));
    }
    return rgb;
}
