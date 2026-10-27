//! Roll-frozen holder exclusions precede base-relative interpretation of per-photo transitions.
use super::*;

#[cfg(feature = "project")]
mod fit;

pub(super) use fit::prepare;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Role {
    Holder,
    BaseCompatible,
    AttenuatedCompatible,
    BrighterThanBase,
    Unknown,
}

struct Scene {
    size: [u32; 2],
    holder: Vec<bool>,
    base: Option<Base>,
}

impl Scene {
    fn fit(captures: &[Capture], model: &RollModel) -> Self {
        Self::fit_refs(&captures.iter().collect::<Vec<_>>(), model)
    }

    fn fit_refs(captures: &[&Capture], model: &RollModel) -> Self {
        assert!(!captures.is_empty());
        let size = captures[0].size;
        assert!(
            captures
                .iter()
                .enumerate()
                .all(|(i, c)| c.size == size && model.owns(i, c))
        );
        let [width, height] = size.map(|v| v as usize);
        let mut votes = vec![0usize; width * height];
        if let Some(upper) = model.upper {
            for capture in captures {
                let dark = |x: usize, y: usize| {
                    (0..3).all(|ch| f64::from(capture.rgb[y * width + x][ch]) <= upper[ch])
                };
                let mut own = vec![false; width * height];
                for y in 0..height {
                    for x in (0..width).take_while(|&x| dark(x, y)) {
                        own[y * width + x] = true;
                    }
                    for x in (0..width).rev().take_while(|&x| dark(x, y)) {
                        own[y * width + x] = true;
                    }
                }
                for x in 0..width {
                    for y in (0..height).take_while(|&y| dark(x, y)) {
                        own[y * width + x] = true;
                    }
                    for y in (0..height).rev().take_while(|&y| dark(x, y)) {
                        own[y * width + x] = true;
                    }
                }
                for (count, is_exterior) in votes.iter_mut().zip(own) {
                    *count += usize::from(is_exterior);
                }
            }
        }
        // Experimental corroboration, not a calibrated probability or complete aperture fit.
        let quorum = 2.max((3 * captures.len()).div_ceil(4));
        let holder: Vec<_> = votes.into_iter().map(|count| count >= quorum).collect();
        let grids: Vec<Vec<_>> = captures
            .iter()
            .map(|capture| {
                let mut grid = vec![];
                for &y in &model.ys {
                    for &x in &model.xs {
                        let index = y as usize * width + x as usize;
                        if !holder[index] {
                            grid.push(capture.rgb[index]);
                        }
                    }
                }
                grid
            })
            .collect();
        let base = if grids.iter().all(|grid| !grid.is_empty()) {
            fit_base(&grids)
        } else {
            None
        };
        Self { size, holder, base }
    }

    fn roles(&self, capture: &Capture) -> Vec<Role> {
        assert_eq!(capture.size, self.size);
        capture
            .rgb
            .iter()
            .zip(&self.holder)
            .map(|(&rgb, &holder)| {
                if holder {
                    return Role::Holder;
                }
                let (Some(base), Some(value)) = (&self.base, logs(rgb)) else {
                    return Role::Unknown;
                };
                let delta: [f64; 3] = std::array::from_fn(|ch| value[ch] - base.mean[ch]);
                let band: [f64; 3] = base.spread.map(|v| 3. * v);
                if (0..3).all(|ch| delta[ch].abs() <= band[ch]) {
                    Role::BaseCompatible
                } else if (0..3).all(|ch| delta[ch] <= band[ch]) {
                    Role::AttenuatedCompatible
                } else if (0..3).all(|ch| delta[ch] >= -band[ch])
                    && (0..3).any(|ch| delta[ch] > band[ch].max(0.5))
                {
                    Role::BrighterThanBase
                } else {
                    Role::Unknown
                }
            })
            .collect()
    }
}
