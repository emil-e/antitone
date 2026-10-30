//! Source-anchored grain spectrum, independent of photographic processing and other effects.
/// Gaussian covariance factors fitted to the scan cohort, with source frames held out.
/// Each factor mixes independent latent fields into RGB; their sum is one continuous spectrum.
#[derive(Debug, Clone, Copy)]
pub struct GrainBasis {
    pub sigma: f64,
    pub factor: [[f64; 3]; 3],
}

pub const GRAIN_BASIS: [GrainBasis; 2] = [
    GrainBasis {
        sigma: 0.24,
        factor: [
            [0.3730124895, 0.5050531591, 0.2495790334],
            [0.6619305266, 0.4772988068, -0.2357789247],
            [1.5330216192, -0.3289776090, 0.0410778103],
        ],
    },
    GrainBasis {
        sigma: 0.68,
        factor: [
            [-0.5272974482, 0.4116058302, 0.2041265846],
            [-0.5838512039, 0.0504305084, -0.3754140180],
            [0.6499508815, 0.3792326835, -0.1716291221],
        ],
    },
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrainParameters {
    pub grain_size: f64,
    pub amplitude: f64,
    pub colour: f64,
}

impl GrainParameters {
    pub fn support(self, _pitch: [f64; 2]) -> crate::processing_support::Support {
        crate::processing_support::Support::NONE
    }
}
#[cfg(any(test, feature = "gpu"))]
impl GrainParameters {
    pub(crate) fn reference_power(self) -> f64 {
        GRAIN_BASIS
            .iter()
            .map(|basis| {
                let variance = (basis.sigma * self.grain_size).powi(2);
                basis.factor[1].iter().map(|v| v * v).sum::<f64>() * variance
                    / (variance + 1. / 12.)
            })
            .sum()
    }

    pub(crate) fn colour_row(self, basis: GrainBasis, channel: usize) -> [f64; 3] {
        // The midpoint reproduces the fitted scan covariance; the upper half increases chroma.
        let colour = 2. * self.colour;
        std::array::from_fn(|i| {
            colour * basis.factor[channel][i] + (1. - colour) * basis.factor[1][i]
        })
    }

    pub(crate) fn covariance(self, footprint: f64) -> [[f64; 3]; 3] {
        let footprint = footprint.max(1.);
        let reference = self.reference_power();
        let mut covariance = [[0.; 3]; 3];
        for basis in GRAIN_BASIS {
            let variance = (basis.sigma * self.grain_size).powi(2);
            let power = variance / (variance + footprint * footprint / 12.) / reference;
            let rows: [[f64; 3]; 3] = std::array::from_fn(|c| self.colour_row(basis, c));
            for r in 0..3 {
                for c in 0..3 {
                    covariance[r][c] +=
                        power * (0..3).map(|i| rows[r][i] * rows[c][i]).sum::<f64>();
                }
            }
        }
        covariance
    }

    pub(crate) fn channel_variance(self, footprint: f64) -> [f64; 3] {
        let covariance = self.covariance(footprint);
        std::array::from_fn(|c| covariance[c][c])
    }
}
