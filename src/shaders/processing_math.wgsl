// Smaller arguments reduce polynomial work without rounding camera values to plain f32.
fn precise_log1p(z: Triple) -> Triple {
    var reduced = z;
    let shift = z.h < -0.25;
    // 1+z = (1 + (1+2z))/2; the atanh argument then has magnitude <= 1/5.
    if shift { reduced = precise_add(number(1.0), precise_mul(number(2.0), z)); }
    let r = precise_div(reduced, precise_add(number(2.0), reduced));
    let r2 = precise_mul(r, r);
    var sum = LOG_ODD[10];
    for (var i = 10u; i > 0u; i--) {
        sum = precise_add(LOG_ODD[i - 1u], precise_mul(r2, sum));
    }
    let result = precise_mul(number(2.0), precise_mul(r, sum));
    if shift { return precise_sub(result, LN2_PRECISE); }
    return result;
}
fn precise_exp(x: Triple) -> Triple {
    let n = round(x.h / LN2_PRECISE.h);
    // Retain n*ln2's residual until subtraction, then evaluate exp(r/2)^2.
    let r = precise_mul(number(0.5), precise_sub(x, multiply(number(n), LN2_PRECISE)));
    var sum = EXP_FACTORIAL[11];
    for (var i = 11u; i > 0u; i--) {
        sum = precise_add(EXP_FACTORIAL[i - 1u], precise_mul(r, sum));
    }
    return precise_mul(precise_mul(sum, sum), number(exp2(n)));
}
fn precise_expm1(x: Triple) -> Triple {
    if abs(x.h) < 0.5 {
        let z = precise_mul(number(0.5), x);
        var sum = EXP_FACTORIAL[12];
        for (var i = 12u; i > 1u; i--) {
            sum = precise_add(EXP_FACTORIAL[i - 1u], precise_mul(z, sum));
        }
        let q = precise_mul(z, sum);
        // expm1(x) = q*(2+q), avoiding cancellation at zero.
        return precise_mul(q, precise_add(number(2.0), q));
    }
    return precise_sub(precise_exp(x), number(1.0));
}
