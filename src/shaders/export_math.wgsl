// Keep the full-series compensated EXR path independent of ordinary-output optimization.
fn precise_log1p(z: Triple) -> Triple {
    let r = precise_div(z, precise_add(number(2.0), z));
    let r2 = precise_mul(r, r);
    var sum = LOG_ODD[14];
    for (var i = 14u; i > 0u; i--) {
        sum = precise_add(LOG_ODD[i - 1u], precise_mul(r2, sum));
    }
    return precise_mul(number(2.0), precise_mul(r, sum));
}
fn precise_exp(x: Triple) -> Triple {
    let n = round(x.h / LN2_PRECISE.h);
    let r = precise_sub(x, precise_mul(number(n), LN2_PRECISE));
    var sum = EXP_FACTORIAL[17];
    for (var i = 17u; i > 0u; i--) {
        sum = precise_add(EXP_FACTORIAL[i - 1u], precise_mul(r, sum));
    }
    return precise_mul(sum, number(exp2(n)));
}
fn precise_expm1(x: Triple) -> Triple {
    if abs(x.h) < 0.5 {
        var sum = EXP_FACTORIAL[17];
        for (var i = 17u; i > 1u; i--) {
            sum = precise_add(EXP_FACTORIAL[i - 1u], precise_mul(x, sum));
        }
        return precise_mul(x, sum);
    }
    return precise_sub(precise_exp(x), number(1.0));
}
