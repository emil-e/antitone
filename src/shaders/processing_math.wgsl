
fn precise_exp(x: Triple) -> Triple {
    let n = round(x.h / LN2_PRECISE.h);
    // Retain n*ln2's residual until subtraction, then evaluate exp(r/2)^2.
    let r = precise_mul(number(0.5), precise_sub(x, multiply(number(n), LN2_PRECISE)));
    var sum = EXP_FACTORIAL[11];
    for(var i = 11u; i > 0u; i--) {
        sum = precise_add(EXP_FACTORIAL[i - 1u], precise_mul(r, sum));
    }
    return precise_mul(precise_mul(sum, sum), number(exp2(n)));
}

