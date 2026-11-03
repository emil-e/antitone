//! Shared two-dimensional sampling of generated exposure tables.
use std::ops::{AddAssign, Div, Mul};

// Mitchell–Netravali B=C=1/3, matching spektrafilm.utils.fast_interp_lut.
fn mitchell(t: f64) -> f64 {
    let x = t.abs();
    if x < 1. {
        ((7. * x - 12.) * x * x + 16. / 3.) / 6.
    } else if x < 2. {
        (((-7. / 3. * x + 12.) * x - 20.) * x + 32. / 3.) / 6.
    } else {
        0.
    }
}

pub(super) fn cubic_2d<T>(values: &[T], size: usize, tc: [f64; 2]) -> T
where
    T: Copy + Default + AddAssign + Mul<f64, Output = T> + Div<f64, Output = T>,
{
    let p = tc.map(|x| x.clamp(0., 1.) * (size - 1) as f64);
    let cell = p.map(|x| (x as usize).min(size - 2));
    let reflect = |i: isize| {
        (if i < 0 {
            -i
        } else if i >= size as isize {
            2 * (size as isize - 1) - i
        } else {
            i
        }) as usize
    };
    let mut result = T::default();
    let mut total = 0.;
    for i in 0..4 {
        for j in 0..4 {
            let weight = mitchell(p[0] - cell[0] as f64 + 1. - i as f64)
                * mitchell(p[1] - cell[1] as f64 + 1. - j as f64);
            result += values
                [reflect(cell[0] as isize - 1 + i) * size + reflect(cell[1] as isize - 1 + j)]
                * weight;
            total += weight;
        }
    }
    result / total
}
