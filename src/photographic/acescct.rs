//! ACEScct look interchange, independent of the photographic contrast coordinate.

const TOE: f64 = 0.0078125;
const SLOPE: f64 = 10.5402377416545;
const OFFSET: f64 = 0.0729055341958355;
const CUT: f64 = 0.155251141552511;

pub(crate) fn encode(x: f64) -> f64 {
    if x <= TOE {
        SLOPE * x + OFFSET
    } else {
        (x.log2() + 9.72) / 17.52
    }
}

pub(crate) fn decode(q: f64) -> f64 {
    // No interchange ceiling: look samples and working RGB stay signed/HDR.
    if q <= CUT {
        (q - OFFSET) / SLOPE
    } else {
        (q * 17.52 - 9.72).exp2()
    }
}
