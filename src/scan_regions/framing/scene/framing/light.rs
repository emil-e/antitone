use super::*;

/// Exact row coverage of known exterior-connected shine-through. Interior reflections/unknown
/// bright samples are not silently promoted to an exterior film cut.
pub(super) struct ExteriorLight {
    size: [u32; 2],
    prefix: Vec<u32>,
    present: bool,
}

impl ExteriorLight {
    pub(super) fn new(capture: &Capture, roles: &[Role]) -> Self {
        let [width, height] = capture.size.map(|value| value as usize);
        let allowed: Vec<_> = roles
            .iter()
            .map(|&role| role == Role::BrighterThanBase)
            .collect();
        let connected = super::super::super::super::connected(&allowed, width, height);
        let mut mask = vec![false; width * height];
        for &index in &connected {
            mask[index] = true;
        }
        let mut prefix = vec![0u32; (width + 1) * height];
        for y in 0..height {
            for x in 0..width {
                prefix[y * (width + 1) + x + 1] =
                    prefix[y * (width + 1) + x] + u32::from(mask[y * width + x]);
            }
        }
        Self {
            size: capture.size,
            prefix,
            present: !connected.is_empty(),
        }
    }

    pub(super) fn intersects(&self, frame: &PictureGeometry) -> bool {
        if !self.present {
            return false;
        }
        let (s, c) = frame.angle.to_radians().sin_cos();
        let width = self.size[0] as usize;
        for row in 0..self.size[1] as usize {
            let y = row as f64 + 0.5;
            let mut low = 0f64;
            let mut high = f64::from(self.size[0]);
            let mut empty = false;
            for (side, bound) in frame.sides.iter().enumerate() {
                let Some(bound) = bound else {
                    continue;
                };
                let (a, b) = if side < 2 { (c, s) } else { (-s, c) };
                let increasing = side == 0 || side == 2;
                if a.abs() < 1e-12 {
                    if (increasing && b * y < *bound) || (!increasing && b * y >= *bound) {
                        empty = true;
                        break;
                    }
                } else {
                    let x = (bound - b * y) / a;
                    if increasing == (a > 0.) {
                        low = low.max(x);
                    } else {
                        high = high.min(x);
                    }
                }
            }
            if empty || high <= low {
                continue;
            }
            let start = (low - 0.5).ceil().clamp(0., width as f64) as usize;
            let end = (high - 0.5).ceil().clamp(0., width as f64) as usize;
            if end > start
                && self.prefix[row * (width + 1) + end] > self.prefix[row * (width + 1) + start]
            {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
