//! Independent f64 oracle: explicit normalized taps, weighted sums and scalar channel loops.
//! Coverage follows the raster contract; native capture flags are not view coverage.
pub(crate) fn apply(
    input: &[[f64; 4]],
    size: [usize; 2],
    sigma: [f64; 2],
    strength: f64,
    coverage: u32,
) -> Vec<[f64; 4]> {
    assert_eq!(input.len(), size[0] * size[1]);
    if strength == 0. {
        return input.to_vec();
    }
    let mut blurred: Vec<_> = input
        .iter()
        .map(|p| [p[0].max(0.), p[1].max(0.), p[2].max(0.), p[3]])
        .collect();
    for axis in 0..2 {
        let radius = (4. * sigma[axis]).ceil() as isize;
        let weights: Vec<_> = (-radius..=radius)
            .map(|i| (-0.5 * (i as f64 / sigma[axis]).powi(2)).exp())
            .collect();
        let previous = blurred;
        blurred = previous.clone();
        for y in 0..size[1] {
            for x in 0..size[0] {
                let index = y * size[0] + x;
                let mut sum = [0.; 3];
                let mut normalization = 0.;
                for (offset, weight) in (-radius..=radius).zip(&weights) {
                    let mut p = [x, y];
                    p[axis] =
                        (p[axis] as isize + offset).clamp(0, size[axis] as isize - 1) as usize;
                    let sample = previous[p[1] * size[0] + p[0]];
                    if coverage != 0 && sample[3] as u32 & coverage == 0 {
                        continue;
                    }
                    normalization += weight;
                    for c in 0..3 {
                        sum[c] += weight * sample[c];
                    }
                }
                if normalization > 0. {
                    for c in 0..3 {
                        blurred[index][c] = sum[c] / normalization;
                    }
                }
            }
        }
    }
    input
        .iter()
        .zip(blurred)
        .map(|(original, blur)| {
            let mut result = *original;
            for (c, ratio) in [1., 0.56, 0.].into_iter().enumerate() {
                let k = strength * ratio;
                result[c] = original[c] + k / (1. + k) * (blur[c] - original[c].max(0.));
            }
            result
        })
        .collect()
}
