//! A roll-pooled sampling interior, not completion of unobserved physical aperture sides.
use super::{
    rectangles::{bounds, polygon, project},
    *,
};

#[derive(Clone, Copy)]
struct Rail {
    low: f64,
    high: f64,
    midpoint: f64,
}

pub(super) struct Shared {
    quad: Quad,
    pub(super) angle: f64,
    rails: [Option<f64>; 4],
}

impl Shared {
    pub(super) fn fit(captures: &[&Capture], upper: Option<[f64; 3]>) -> Option<Self> {
        let upper = upper?;
        if captures.len() < 2 {
            return None;
        }
        let size = captures[0].size;
        let [w, h] = size.map(|v| v as usize);
        let mut votes = vec![0usize; w * h];
        for capture in captures {
            let dark = |x: usize, y: usize| {
                (0..3).all(|ch| f64::from(capture.rgb[y * w + x][ch]) <= upper[ch])
            };
            let mut exterior = vec![false; w * h];
            // Terminal runs stop at the first transmission. Interior darkness is not a
            // holder exclusion unless it reaches an edge, and must recur across photos.
            for y in 0..h {
                for x in (0..w).take_while(|&x| dark(x, y)) {
                    exterior[y * w + x] = true;
                }
                for x in (0..w).rev().take_while(|&x| dark(x, y)) {
                    exterior[y * w + x] = true;
                }
            }
            for x in 0..w {
                for y in (0..h).take_while(|&y| dark(x, y)) {
                    exterior[y * w + x] = true;
                }
                for y in (0..h).rev().take_while(|&y| dark(x, y)) {
                    exterior[y * w + x] = true;
                }
            }
            for (votes, outside) in votes.iter_mut().zip(exterior) {
                *votes += usize::from(outside);
            }
        }
        // Corroborated exterior is excluded conservatively, not merely where a
        // median photo is black. A single scene cannot redefine the shared opening.
        let quorum = 2.max(captures.len().div_ceil(4));
        let outside: Vec<_> = votes.into_iter().map(|v| v >= quorum).collect();
        if !outside.iter().any(|&v| v) {
            return None;
        }
        let angle = pose(size, &outside);
        let rails = terminal_rails(size, angle, 4., |x, y| outside[y * w + x])
            .map(|r| r.map(|r| r.midpoint));
        // Opposing sides assembled from different, incompatible captures are not a
        // common aperture. Require two captures to corroborate a visible pair.
        for pair in [[0, 1], [2, 3]] {
            if pair.iter().all(|&s| rails[s].is_some()) {
                let matches = captures
                    .iter()
                    .filter(|c| {
                        // Corroboration tolerates the same bounded raggedness as
                        // local holder fitting; rigid-motion evidence stays stricter.
                        let own = terminal_rails(size, angle, 16., |x, y| {
                            (0..3).all(|ch| f64::from(c.rgb[y * w + x][ch]) <= upper[ch])
                        });
                        pair.iter().all(|&s| {
                            own[s].is_some_and(|r| {
                                let shared = rails[s].unwrap();
                                shared >= r.low - 1. && shared <= r.high + 1.
                            })
                        })
                    })
                    .count();
                if matches < 2 {
                    return None;
                }
            }
        }
        // A one-cell neighbourhood covers raster/rotation uncertainty. This is a
        // spatial safety margin, not a change to the original numerical sample grid.
        let quad = inscribe(size, angle, |p| {
            let [x, y] = p.map(|v| v.floor() as isize);
            (-1..=1).all(|dy| {
                (-1..=1).all(|dx| {
                    let (x, y) = (x + dx, y + dy);
                    x >= 0
                        && y >= 0
                        && x < w as isize
                        && y < h as isize
                        && !outside[y as usize * w + x as usize]
                })
            })
        })?;
        Some(Self { quad, angle, rails })
    }

    pub(super) fn constrain(
        &self,
        size: [u32; 2],
        local: Quad,
        observed: [bool; 4],
        own_rails: [Option<f64>; 4],
    ) -> (Option<Quad>, bool) {
        let mut shift = [0.; 2];
        for (axis, pair) in [[0, 1], [2, 3]].into_iter().enumerate() {
            if let (Some(a), Some(b), Some(c), Some(d)) = (
                own_rails[pair[0]],
                own_rails[pair[1]],
                self.rails[pair[0]],
                self.rails[pair[1]],
            ) {
                let (a, b) = (a - c, b - d);
                // A scene can deepen a terminal run but cannot move the opposite
                // physical rail outward with it. Paired same-direction motion can.
                if a * b > 0. && a.abs().min(b.abs()) > 2. && (a - b).abs() <= 4. {
                    shift[axis] = (a + b) / 2.;
                }
            }
        }
        let moved = shift.iter().any(|&v| v != 0.);
        let shift = project(shift, -self.angle);
        let quad = self.quad.map(|p| [p[0] + shift[0], p[1] + shift[1]]);
        let allowed = |p: [f64; 2]| {
            observed.iter().enumerate().all(|(side, &seen)| {
                if !seen {
                    return true;
                }
                let edge = [3, 1, 0, 2][side];
                let a = local[edge];
                let b = local[(edge + 1) % 4];
                (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-9
            })
        };
        if quad.iter().all(|&p| allowed(p)) {
            return (Some(quad), moved);
        }
        // Without paired motion, local evidence may only tighten the pooled interior.
        let q = inscribe(size, self.angle, |p| inside(p, &quad) && allowed(p));
        let exceptional = moved
            || q.is_none_or(|q| {
                q.iter()
                    .zip(quad)
                    .any(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) > 8.)
            });
        (q, exceptional)
    }

    pub(super) fn refine(&self, size: [u32; 2], active: SourceRegion, areas: &mut [Areas]) {
        let normals: Vec<_> = areas
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.opening_shared && !a.opening_exception && !a.opening_edited && a.opening.is_some()
            })
            .map(|(i, a)| (i, a.opening.unwrap()))
            .collect();
        if normals.len() < 2 {
            return;
        }
        let source = |p: [f64; 2]| {
            [
                f64::from(active.x) + p[0] * f64::from(active.width) / f64::from(size[0]),
                f64::from(active.y) + p[1] * f64::from(active.height) / f64::from(size[1]),
            ]
        };
        // Fine measurements strengthen the common safety envelope, rather than
        // presenting native threshold/raster jitter as a different holder per photo.
        let common = if normals.iter().all(|(_, q)| q == &normals[0].1) {
            Some(normals[0].1)
        } else {
            inscribe(size, self.angle, |p| {
                normals.iter().all(|(_, q)| inside(source(p), q))
            })
            .map(|q| q.map(source))
        };
        for (i, _) in normals {
            areas[i].opening = common;
            areas[i].refit_picture(active);
        }
    }
}

pub(super) fn rail(
    size: [u32; 2],
    angle: f64,
    side: usize,
    points: impl Iterator<Item = [f64; 2]>,
) -> Option<f64> {
    fitted_rail(size, angle, side, points, 4.).map(|r| r.midpoint)
}
fn fitted_rail(
    size: [u32; 2],
    angle: f64,
    side: usize,
    points: impl Iterator<Item = [f64; 2]>,
    spread: f64,
) -> Option<Rail> {
    let along = usize::from(side < 2);
    let extent = f64::from(size[along]);
    let points: Vec<_> = points
        .filter(|p| p[along] >= extent * 0.25 && p[along] <= extent * 0.75)
        .collect();
    if points.len() < 8 {
        return None;
    }
    let lo = points
        .iter()
        .map(|p| p[along])
        .fold(f64::INFINITY, f64::min);
    let hi = points
        .iter()
        .map(|p| p[along])
        .fold(f64::NEG_INFINITY, f64::max);
    if hi - lo < extent * 0.4 {
        return None;
    }
    let values: Vec<_> = points
        .into_iter()
        .map(|p| project(p, angle)[1 - along])
        .collect();
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (hi - lo <= spread).then(|| Rail {
        low: lo,
        high: hi,
        midpoint: median(values),
    })
}
fn terminal_rails(
    size: [u32; 2],
    angle: f64,
    spread: f64,
    dark: impl Fn(usize, usize) -> bool,
) -> [Option<Rail>; 4] {
    let [w, h] = size.map(|v| v as usize);
    std::array::from_fn(|side| {
        let (along, cross) = if side < 2 { (h, w) } else { (w, h) };
        let points = (along / 4..3 * along / 4).filter_map(|i| {
            let depth = (0..cross / 3)
                .take_while(|&d| {
                    let (x, y) = match side {
                        0 => (d, i),
                        1 => (w - 1 - d, i),
                        2 => (i, d),
                        _ => (i, h - 1 - d),
                    };
                    dark(x, y)
                })
                .count();
            if depth == 0 || depth == cross / 3 {
                return None;
            }
            let normal = if side == 0 || side == 2 {
                depth as f64 - 0.5
            } else {
                (cross - depth) as f64 + 0.5
            };
            Some(if side < 2 {
                [normal, i as f64 + 0.5]
            } else {
                [i as f64 + 0.5, normal]
            })
        });
        fitted_rail(size, angle, side, points, spread)
    })
}
fn pose(size: [u32; 2], outside: &[bool]) -> f64 {
    let [w, h] = size.map(|v| v as usize);
    let mut angles = vec![];
    for side in 0..4 {
        let (along, cross) = if side < 2 { (h, w) } else { (w, h) };
        let mut points = vec![];
        for i in along / 4..3 * along / 4 {
            let depth = (0..cross)
                .take_while(|&d| {
                    let (x, y) = match side {
                        0 => (d, i),
                        1 => (w - 1 - d, i),
                        2 => (i, d),
                        _ => (i, h - 1 - d),
                    };
                    outside[y * w + x]
                })
                .count();
            if depth > 0 && depth < cross / 3 {
                points.push((
                    i as f64 + 0.5,
                    if side == 0 || side == 2 {
                        depth as f64
                    } else {
                        (cross - depth) as f64
                    },
                ));
            }
        }
        let mut slopes = vec![];
        for (i, a) in points.iter().enumerate().step_by(4) {
            for b in points[i + 1..].iter().step_by(4) {
                if b.0 - a.0 >= along as f64 / 8. {
                    slopes.push((b.1 - a.1) / (b.0 - a.0));
                }
            }
        }
        if !slopes.is_empty() {
            let angle = median(slopes).atan().to_degrees() * if side < 2 { -1. } else { 1. };
            if angle.abs() <= 6. {
                angles.push(angle);
            }
        }
    }
    if angles.is_empty() {
        0.
    } else {
        median(angles)
    }
}

/// Largest rectangular interior in the holder basis, containing the capture centre.
/// Corners and terminal intrusions must fit too; four infinite fitted rails are insufficient.
fn inscribe(size: [u32; 2], angle: f64, mut allowed: impl FnMut([f64; 2]) -> bool) -> Option<Quad> {
    let [w, h] = size.map(|v| v as usize);
    let origin = bounds(size, angle, 0.);
    let mut heights = vec![0usize; w];
    let mut best = (0usize, [0usize; 4]);
    for y in 0..h {
        for (x, height) in heights.iter_mut().enumerate() {
            let fits = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                .into_iter()
                .all(|[dx, dy]| {
                    allowed(project(
                        [origin[0] + x as f64 + dx, origin[2] + y as f64 + dy],
                        -angle,
                    ))
                });
            *height = if fits { *height + 1 } else { 0 };
        }
        let mut stack: Vec<(usize, usize)> = vec![];
        for x in 0..=w {
            let height = heights.get(x).copied().unwrap_or(0);
            let mut left = x;
            while let Some(&(start, previous)) = stack.last() {
                if previous <= height {
                    break;
                }
                stack.pop();
                left = start;
                let top = y + 1 - previous;
                if start <= w / 2 && x > w / 2 && top <= h / 2 && y >= h / 2 {
                    let area = (x - start) * previous;
                    let rectangle = [start, x, top, y + 1];
                    if area > best.0 || area == best.0 && rectangle < best.1 {
                        best = (area, rectangle);
                    }
                }
            }
            if height > 0 && stack.last().is_none_or(|&(_, h)| h < height) {
                stack.push((left, height));
            }
        }
    }
    if best.0 == 0 {
        return None;
    }
    let [l, r, t, b] = best.1;
    polygon(
        [
            origin[0] + l as f64,
            origin[0] + r as f64,
            origin[2] + t as f64,
            origin[2] + b as f64,
        ],
        angle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_inscribed_cell_must_fit_in_full_not_only_at_its_centre() {
        let allowed = |p: [f64; 2]| p[0] + p[1] >= 9.3;
        let q = inscribe([32, 32], 0., allowed).unwrap();
        assert!(
            q.into_iter().all(allowed),
            "cell corners protrude outside the measured half-plane: {q:?}"
        );
    }
}
