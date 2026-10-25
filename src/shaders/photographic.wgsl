const CCT_SLOPE: Triple = Triple(10.540237426757812, 3.1489668117501424e-7, 7.105427357601002e-15);
const CCT_OFFSET: Triple = Triple(0.072905533015728, 1.180107456200119e-9, 4.163336342344337e-17);
const CCT_SCALE: Triple = Triple(
        17.520000457763672,
        -4.577636616431846e-7,
        -1.0658141036401503e-14,
    );
const CCT_LOG_SCALE: Triple = Triple(
        12.143939018249512,
        -4.148392633851472e-7,
        -7.105427357601002e-15,
    );

fn precise_le(a: Triple, b: Triple) -> bool {
    return a.h < b.h || (a.h == b.h && (a.l < b.l || (a.l == b.l && a.t <= b.t)));
}

fn creative_constant(row: u32, column: u32) -> Triple {
    return Triple(
            parameters.creative[row][column],
            parameters.creative_low[row][column],
            parameters.creative_tail[row][column],
        );
}

