//! Research-only robust log-illumination fit. Not a deployed correction/filter policy.
#[derive(Clone)]
pub struct Surface {
    degree: usize,
    coefficients: Vec<f64>,
}
fn terms(x: f64, y: f64, degree: usize) -> Vec<f64> {
    let mut result = Vec::new();
    for order in 1..=degree {
        for j in 0..=order {
            result.push(x.powi((order - j) as i32) * y.powi(j as i32));
        }
    }
    result
}
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for k in 0..n {
        let pivot = (k..n)
            .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
            .unwrap();
        assert!(a[pivot][k].abs() > 1e-14, "singular research fit");
        a.swap(k, pivot);
        b.swap(k, pivot);
        for i in k + 1..n {
            let factor = a[i][k] / a[k][k];
            for j in k + 1..n {
                a[i][j] -= factor * a[k][j];
            }
            b[i] -= factor * b[k];
        }
    }
    let mut x = vec![0.; n];
    for i in (0..n).rev() {
        x[i] = (b[i] - (i + 1..n).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    x
}
impl Surface {
    /// x/y are centered, normalized source coordinates; z is ln(Y / measured centre Y).
    /// No constant term: the fitted illumination equals the measured reference at image centre.
    pub fn fit(samples: &[[f64; 3]], degree: usize) -> Self {
        let basis: Vec<_> = samples.iter().map(|p| terms(p[0], p[1], degree)).collect();
        let n = basis[0].len();
        let mut weights = vec![1.; samples.len()];
        let mut coefficients = vec![0.; n];
        for _ in 0..6 {
            let mut a = vec![vec![0.; n]; n];
            let mut b = vec![0.; n];
            for ((p, t), weight) in samples.iter().zip(&basis).zip(&weights) {
                for i in 0..n {
                    b[i] += weight * t[i] * p[2];
                    for j in 0..=i {
                        a[i][j] += weight * t[i] * t[j];
                    }
                }
            }
            for i in 0..n {
                for j in 0..i {
                    a[j][i] = a[i][j];
                }
            }
            coefficients = solve(a, b);
            let residuals: Vec<_> = samples
                .iter()
                .zip(&basis)
                .map(|(p, t)| {
                    (p[2] - t.iter().zip(&coefficients).map(|(v, c)| v * c).sum::<f64>()).abs()
                })
                .collect();
            let mut sorted = residuals.clone();
            sorted.sort_by(f64::total_cmp);
            let cutoff = (3. * sorted[sorted.len() / 2]).max(1e-5);
            for (weight, residual) in weights.iter_mut().zip(residuals) {
                *weight = if residual <= cutoff {
                    1.
                } else {
                    cutoff / residual
                };
            }
        }
        Self {
            degree,
            coefficients,
        }
    }
    pub fn log_relative_light(&self, x: f64, y: f64) -> f64 {
        terms(x, y, self.degree)
            .iter()
            .zip(&self.coefficients)
            .map(|(v, c)| v * c)
            .sum()
    }
}
/// Separable local quadratic Gaussian regression in log light. Rectangle-only support.
/// Unlike a truncated blur, regression preserves linear/quadratic edge slopes.
pub struct LocalSurface {
    values: Vec<f64>,
    width: usize,
    height: usize,
}
fn kernel(at: usize, size: usize, sigma: f64, parity: Option<usize>) -> Vec<(usize, f64)> {
    let radius = (3. * sigma).ceil() as usize;
    let first = at.saturating_sub(radius);
    let end = (at + radius + 1).min(size);
    let mut gram = vec![vec![0.; 3]; 3];
    let mut samples = Vec::new();
    for index in first..end {
        if parity.is_some_and(|p| index % 2 != p) {
            continue;
        }
        let d = (index as f64 - at as f64) / sigma;
        let t = [1., d, d * d];
        let w = (-0.5 * d * d).exp();
        for i in 0..3 {
            for j in 0..3 {
                gram[i][j] += w * t[i] * t[j];
            }
        }
        samples.push((index, t, w));
    }
    let intercept = solve(gram, vec![1., 0., 0.]);
    samples
        .into_iter()
        .map(|(index, t, w)| {
            (
                index,
                w * t.iter().zip(&intercept).map(|(a, b)| a * b).sum::<f64>(),
            )
        })
        .collect()
}
fn fit_plane(points: &[[f64; 3]]) -> Vec<f64> {
    let mut gram = vec![vec![0.; 3]; 3];
    let mut rhs = vec![0.; 3];
    for p in points {
        let t = [1., p[0], p[1]];
        for i in 0..3 {
            rhs[i] += t[i] * p[2];
            for j in 0..3 {
                gram[i][j] += t[i] * t[j];
            }
        }
    }
    solve(gram, rhs)
}
impl LocalSurface {
    /// Training is checkerboard-even only; held-out values never enter median repair or smoothing.
    pub fn fit(values: &[f64], width: usize, height: usize, sigma: f64) -> Self {
        let mut repaired = values.to_vec();
        for y in 0..height {
            for x in 0..width {
                if (x + y) % 2 != 0 {
                    continue;
                }
                let mut neighbors = Vec::new();
                for yy in y.saturating_sub(2)..(y + 3).min(height) {
                    for xx in x.saturating_sub(2)..(x + 3).min(width) {
                        if (xx + yy) % 2 == 0 {
                            neighbors.push([
                                xx as f64 - x as f64,
                                yy as f64 - y as f64,
                                values[yy * width + xx],
                            ]);
                        }
                    }
                }
                let plane = fit_plane(&neighbors);
                let mut deviations: Vec<_> = neighbors
                    .iter()
                    .map(|p| (p[2] - plane[0] - plane[1] * p[0] - plane[2] * p[1]).abs())
                    .collect();
                deviations.sort_by(f64::total_cmp);
                if (values[y * width + x] - plane[0]).abs()
                    > (6. * deviations[deviations.len() / 2]).max(0.006)
                {
                    neighbors.retain(|p| p[0] != 0. || p[1] != 0.);
                    repaired[y * width + x] = fit_plane(&neighbors)[0];
                }
            }
        }
        let xkernels: Vec<_> = (0..2)
            .map(|parity| {
                (0..width)
                    .map(|x| kernel(x, width, sigma, Some(parity)))
                    .collect::<Vec<_>>()
            })
            .collect();
        let ykernels: Vec<_> = (0..height)
            .map(|y| kernel(y, height, sigma, None))
            .collect();
        let mut horizontal = vec![0.; width * height];
        for y in 0..height {
            for x in 0..width {
                horizontal[y * width + x] = xkernels[y % 2][x]
                    .iter()
                    .map(|(xx, w)| repaired[y * width + xx] * w)
                    .sum();
            }
        }
        let mut smoothed = vec![0.; width * height];
        for y in 0..height {
            for x in 0..width {
                smoothed[y * width + x] = ykernels[y]
                    .iter()
                    .map(|(yy, w)| horizontal[yy * width + x] * w)
                    .sum();
            }
        }
        Self {
            values: smoothed,
            width,
            height,
        }
    }
    pub fn at(&self, x: usize, y: usize) -> f64 {
        self.values[y * self.width + x]
    }
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        assert!(
            x >= 0. && y >= 0. && x <= (self.width - 1) as f64 && y <= (self.height - 1) as f64
        );
        let x0 = (x.floor() as usize).min(self.width - 2);
        let y0 = (y.floor() as usize).min(self.height - 2);
        let dx = x - x0 as f64;
        let dy = y - y0 as f64;
        (1. - dy) * ((1. - dx) * self.at(x0, y0) + dx * self.at(x0 + 1, y0))
            + dy * ((1. - dx) * self.at(x0, y0 + 1) + dx * self.at(x0 + 1, y0 + 1))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_surface_preserves_boundary_curvature_and_does_not_consume_held_out_data() {
        let (width, height) = (61, 41);
        let light = |x: f64, y: f64| {
            0.001 * x - 0.002 * y - 0.00008 * x * x - 0.0001 * y * y + 0.0002 * x * y
        };
        let values: Vec<_> = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    if (x + y) % 2 == 0 {
                        light(x as f64, y as f64)
                    } else {
                        999.
                    }
                })
            })
            .collect();
        let fit = LocalSurface::fit(&values, width, height, 4.);
        for y in 0..height {
            for x in 0..width {
                assert!(
                    (fit.at(x, y) - light(x as f64, y as f64)).abs() < 1e-10,
                    "x={x} y={y} got={} expected={}",
                    fit.at(x, y),
                    light(x as f64, y as f64)
                );
            }
        }
    }
    #[test]
    fn nonradial_smooth_field_recovers_held_out_samples_with_dust_and_exact_center_anchor() {
        let light =
            |x: f64, y: f64| 0.025 * x - 0.01 * y - 0.08 * x * x - 0.15 * y * y + 0.02 * x * y;
        let mut samples = Vec::new();
        for y in 0..41 {
            for x in 0..61 {
                if (x + y) % 2 != 0 {
                    continue;
                }
                let xx = x as f64 / 30. - 1.;
                let yy = y as f64 / 20. - 1.;
                let dust = if (x + 3 * y) % 97 == 0 { -0.3 } else { 0. };
                samples.push([xx, yy, light(xx, yy) + dust]);
            }
        }
        let fit = Surface::fit(&samples, 2);
        assert_eq!(fit.log_relative_light(0., 0.), 0.);
        for y in 0..40 {
            for x in 0..60 {
                let xx = (x as f64 + 0.5) / 30. - 1.;
                let yy = (y as f64 + 0.5) / 20. - 1.;
                assert!((fit.log_relative_light(xx, yy) - light(xx, yy)).abs() < 2e-5);
            }
        }
    }
}
