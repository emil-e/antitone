use super::require;
use crate::Error;

/// Monotone C1 run-centre interpretation of the quantized source shapers.
/// The grey knot retains its original source ordinate; bounds use rational tails.
#[derive(Clone, Debug)]
pub(crate) struct Curve {
    pub knots: Vec<[f64; 3]>,
    pub low: f64,
    pub high: f64,
}
impl Curve {
    pub fn from_runs(values: &[u16], anchor: f64) -> Result<Self, Error> {
        require(values.len() == 1024, "invalid source shaper dimensions")?;
        let values = values
            .iter()
            .map(|&x| f64::from(x).clamp(32.0, 960.0))
            .collect::<Vec<_>>();
        require(
            values.windows(2).all(|w| w[0] <= w[1])
                && anchor.is_finite()
                && (0.0..1023.0).contains(&anchor),
            "invalid source shaper",
        )?;
        let mut nodes = Vec::new();
        let mut start = 0;
        for end in 1..=values.len() {
            if end == values.len() || values[end] != values[start] {
                if values[start] > 32.0 && values[start] < 960.0 {
                    nodes.push([(start + end - 1) as f64 / 2.0, values[start], 0.0]);
                }
                start = end;
            }
        }
        let i = anchor as usize;
        let y = values[i] + (values[i + 1] - values[i]) * (anchor - i as f64);
        nodes.retain(|p| p[1] != y);
        nodes.push([anchor, y, 0.0]);
        nodes.sort_by(|a, b| a[0].total_cmp(&b[0]));
        require(
            nodes.len() >= 2
                && nodes
                    .windows(2)
                    .all(|w| w[0][0] < w[1][0] && w[0][1] < w[1][1]),
            "invalid shaper run centres",
        )?;
        let h = nodes
            .windows(2)
            .map(|w| w[1][0] - w[0][0])
            .collect::<Vec<_>>();
        let d = nodes
            .windows(2)
            .zip(&h)
            .map(|(w, h)| (w[1][1] - w[0][1]) / h)
            .collect::<Vec<_>>();
        nodes[0][2] = d[0];
        for i in 1..nodes.len() - 1 {
            let a = 2.0 * h[i] + h[i - 1];
            let b = h[i] + 2.0 * h[i - 1];
            nodes[i][2] = (a + b) / (a / d[i - 1] + b / d[i]);
        }
        let last = nodes.len() - 1;
        nodes[last][2] = d[last - 1];
        let curve = Self {
            knots: nodes,
            low: 32.0,
            high: 960.0,
        };
        curve.validate()?;
        Ok(curve)
    }
    pub fn validate(&self) -> Result<(), Error> {
        require(
            (2..=1025).contains(&self.knots.len())
                && self.low.is_finite()
                && self.high.is_finite()
                && 0.0 <= self.low
                && self.low < self.high
                && self.high <= 1023.0,
            "invalid shaper bounds",
        )?;
        require(
            self.knots.iter().all(|p| {
                p.iter().all(|x| x.is_finite() && (*x as f32).is_finite())
                    && (0.0..=1023.0).contains(&p[0])
                    && p[2] > 0.0
                    && (p[2] as f32) > 0.0
                    && self.low < p[1]
                    && p[1] < self.high
                    && (self.low as f32) < p[1] as f32
                    && (p[1] as f32) < self.high as f32
            }),
            "invalid shaper coefficients",
        )?;
        for w in self.knots.windows(2) {
            let (a, z) = (w[0], w[1]);
            let h = z[0] - a[0];
            let dy = z[1] - a[1];
            require(
                h > 0.0 && dy > 0.0 && (a[0] as f32) < z[0] as f32 && (a[1] as f32) < z[1] as f32,
                "unordered or unrepresentable shaper knots",
            )?;
            let b = h * a[2];
            let q = h * z[2];
            let c = 3.0 * dy - 2.0 * b - q;
            let d = -2.0 * dy + b + q;
            let derivative = |t: f64| b + 2.0 * c * t + 3.0 * d * t * t;
            let vertex = if d > 0.0 {
                (-c / (3.0 * d)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            require(
                [derivative(0.0), derivative(1.0), derivative(vertex)]
                    .into_iter()
                    .all(|x| x.is_finite() && x >= -1e-12),
                "nonmonotone shaper segment",
            )?;
        }
        Ok(())
    }
    pub fn sample(&self, x: f64) -> f64 {
        let first = self.knots[0];
        let last = self.knots[self.knots.len() - 1];
        if x < first[0] {
            let gap = first[1] - self.low;
            return self.low + gap / (1.0 + first[2] * (first[0] - x) / gap);
        }
        if x >= last[0] {
            let gap = self.high - last[1];
            return self.high - gap / (1.0 + last[2] * (x - last[0]) / gap);
        }
        let i = self.knots.partition_point(|p| p[0] <= x) - 1;
        let (a, z) = (self.knots[i], self.knots[i + 1]);
        let h = z[0] - a[0];
        let t = (x - a[0]) / h;
        let dy = z[1] - a[1];
        let b = h * a[2];
        let q = h * z[2];
        let c = 3.0 * dy - 2.0 * b - q;
        let d = -2.0 * dy + b + q;
        a[1] + t * (b + t * (c + t * d))
    }
}
