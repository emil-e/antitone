//! Sampling of build-time evaluations and disposable, on-demand spectral projections.
use glam::DVec3;

pub(super) struct CubicGrid {
    pub size: usize,
    pub min: DVec3,
    pub max: DVec3,
    pub values: Vec<DVec3>,
}

impl CubicGrid {
    pub fn sample(&self, value: DVec3) -> DVec3 {
        let position = (value.clamp(self.min, self.max) - self.min) / (self.max - self.min)
            * (self.size - 1) as f64;
        let mut starts = [0; 3];
        let mut weights = [[0.; 4]; 3];
        for c in 0..3 {
            let cell = (position[c] as usize).min(self.size - 2);
            let start = cell.saturating_sub(1).min(self.size - 4);
            starts[c] = start;
            for (i, weight) in weights[c].iter_mut().enumerate() {
                *weight = 1.;
                for j in 0..4 {
                    if i != j {
                        *weight *= (position[c] - (start + j) as f64) / (i as f64 - j as f64);
                    }
                }
            }
        }
        let mut result = DVec3::ZERO;
        for i in 0..4 {
            for j in 0..4 {
                for k in 0..4 {
                    let index =
                        ((starts[0] + i) * self.size + starts[1] + j) * self.size + starts[2] + k;
                    result += self.values[index] * (weights[0][i] * weights[1][j] * weights[2][k]);
                }
            }
        }
        result
    }
}

pub(super) fn bilinear(values: &[DVec3], size: usize, tc: [f64; 2]) -> DVec3 {
    let position = tc.map(|x| x.clamp(0., 1.) * (size - 1) as f64);
    let cell = position.map(|x| (x as usize).min(size - 2));
    let f = [position[0] - cell[0] as f64, position[1] - cell[1] as f64];
    let a = values[cell[0] * size + cell[1]].lerp(values[cell[0] * size + cell[1] + 1], f[1]);
    let b = values[(cell[0] + 1) * size + cell[1]]
        .lerp(values[(cell[0] + 1) * size + cell[1] + 1], f[1]);
    a.lerp(b, f[0])
}
