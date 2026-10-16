use super::material::Selection;
use super::*;
use crate::region_timing::timed;

#[derive(Clone)]
struct Run {
    points: Vec<([f64; 2], bool)>,
    angle: f64,
    rays: usize,
}

pub(super) fn project([x, y]: [f64; 2], angle: f64) -> [f64; 2] {
    let (s, c) = angle.to_radians().sin_cos();
    [c * x + s * y, -s * x + c * y]
}

pub(super) fn bounds(size: [u32; 2], angle: f64, inset: f64) -> [f64; 4] {
    let [w, h] = size.map(f64::from);
    let [x, y] = project([w / 2., h / 2.], angle);
    [
        x - w / 2. + inset,
        x + w / 2. - inset,
        y - h / 2. + inset,
        y + h / 2. - inset,
    ]
}

pub(super) fn polygon(b: [f64; 4], angle: f64) -> Option<Quad> {
    let [l, r, t, bot] = b;
    (l < r && t < bot).then(|| [[l, t], [r, t], [r, bot], [l, bot]].map(|p| project(p, -angle)))
}

impl PictureGeometry {
    /// Complete independent frame in source pixels, without opening or coverage limits.
    /// Side presence alone does not establish strength; consumers must check provenance.
    pub fn frame(&self, active: SourceRegion) -> Option<Quad> {
        active.validate().ok()?;
        if self.size.contains(&0) || !self.angle.is_finite() || self.angle.abs() > 6. {
            return None;
        }
        let [Some(l), Some(r), Some(t), Some(b)] = self.sides else {
            return None;
        };
        if [l, r, t, b].iter().any(|v| !v.is_finite()) {
            return None;
        }
        polygon([l, r, t, b], self.angle).map(|quad| {
            quad.map(|p| {
                [
                    f64::from(active.x) + p[0] * f64::from(active.width) / f64::from(self.size[0]),
                    f64::from(active.y) + p[1] * f64::from(active.height) / f64::from(self.size[1]),
                ]
            })
        })
    }

    pub(super) fn fit(&self, active: SourceRegion, opening: Quad) -> Option<Quad> {
        let scale = (f64::from(self.size[0]) / 768.).max(f64::from(self.size[1]) / 512.);
        let mut b = bounds(self.size, self.angle, 2. * scale);
        let projected = opening.map(|p| {
            project(
                [
                    (p[0] - f64::from(active.x)) * f64::from(self.size[0])
                        / f64::from(active.width),
                    (p[1] - f64::from(active.y)) * f64::from(self.size[1])
                        / f64::from(active.height),
                ],
                self.angle,
            )
        });
        for (side, value) in b.iter_mut().enumerate() {
            let axis = usize::from(side >= 2);
            if side == 0 || side == 2 {
                *value = value.max(
                    projected
                        .iter()
                        .map(|p| p[axis])
                        .fold(f64::INFINITY, f64::min),
                );
                if let Some(bound) = self.sides[side] {
                    *value = value.max(bound);
                }
            } else {
                *value = value.min(
                    projected
                        .iter()
                        .map(|p| p[axis])
                        .fold(f64::NEG_INFINITY, f64::max),
                );
                if let Some(bound) = self.sides[side] {
                    *value = value.min(bound);
                }
            }
        }
        polygon(b, self.angle).map(|q| {
            q.map(|p| {
                [
                    f64::from(active.x) + p[0] * f64::from(active.width) / f64::from(self.size[0]),
                    f64::from(active.y) + p[1] * f64::from(active.height) / f64::from(self.size[1]),
                ]
            })
        })
    }
}

fn runs(
    selection: &Selection,
    size: [u32; 2],
    measure: [u32; 2],
    rgb: &[[u16; 3]],
) -> [Option<Run>; 4] {
    std::array::from_fn(|side| {
        let upper = selection.upper?;
        let axis = usize::from(side < 2);
        let along = measure[axis];
        let cross = measure[1 - axis];
        let centres = if side < 2 {
            &selection.material.xs
        } else {
            &selection.material.ys
        };
        let cell = centres
            .windows(2)
            .map(|v| v[1] - v[0])
            .max()
            .unwrap_or(size[1 - axis]) as f64;
        let mut points = vec![];
        for i in 0..along {
            let mut end = 0;
            let limit = (cross / 3).max(1);
            for d in 0..limit {
                let [x, y] = match side {
                    0 => [d, i],
                    1 => [measure[0] - 1 - d, i],
                    2 => [i, d],
                    _ => [i, measure[1] - 1 - d],
                };
                if !(0..3).all(|ch| {
                    f64::from(rgb[y as usize * measure[0] as usize + x as usize][ch]) <= upper[ch]
                }) {
                    break;
                }
                end = d + 1;
            }
            if end == 0 || end == limit {
                continue;
            }
            let p = match side {
                0 => [f64::from(end) - 0.5, f64::from(i) + 0.5],
                1 => [f64::from(measure[0] - end) + 0.5, f64::from(i) + 0.5],
                2 => [f64::from(i) + 0.5, f64::from(end) - 0.5],
                _ => [f64::from(i) + 0.5, f64::from(measure[1] - end) + 0.5],
            };
            let p = std::array::from_fn(|a| p[a] * f64::from(size[a]) / f64::from(measure[a]));
            let mut supported = true;
            if f64::from(end) * f64::from(size[1 - axis]) / f64::from(cross) > 2. * cell {
                let mut out = p;
                out[1 - axis] += if side == 0 || side == 2 { -cell } else { cell };
                supported = selection.material.label(out) == b'H';
            }
            points.push((p, supported));
        }
        if points.len() < 8.max((along as usize).div_ceil(2)) {
            return None;
        }
        let mut bins: [Vec<[f64; 2]>; 32] = std::array::from_fn(|_| vec![]);
        for (p, _) in &points {
            bins[((p[axis] / f64::from(size[axis]) * 32.) as usize).min(31)].push(*p);
        }
        let medians: Vec<[f64; 2]> = bins
            .iter()
            .filter(|v| !v.is_empty())
            .map(|v| {
                [
                    median(v.iter().map(|p| p[0])),
                    median(v.iter().map(|p| p[1])),
                ]
            })
            .collect();
        let mut slopes = vec![];
        for (i, a) in medians.iter().enumerate() {
            for b in &medians[i + 1..] {
                let da = b[axis] - a[axis];
                if da.abs() > f64::from(size[axis]) / 8. {
                    slopes.push((b[1 - axis] - a[1 - axis]) / da);
                }
            }
        }
        if slopes.is_empty() {
            return None;
        }
        let angle = median(slopes).atan().to_degrees() * if side < 2 { -1. } else { 1. };
        Some(Run {
            points,
            angle,
            rays: along as usize,
        })
    })
}

fn opening_side(
    side: usize,
    run: Option<&Run>,
    angle: f64,
    scale: f64,
    coverage: f64,
) -> (f64, bool) {
    let Some(run) = run else {
        return (coverage, false);
    };
    if run.points.len() < 8.max(run.rays.div_ceil(2))
        || run.points.iter().any(|(_, ok)| !*ok)
        || (run.angle - angle).abs() > 2.
    {
        return (coverage, false);
    }
    let axis = usize::from(side >= 2);
    let values: Vec<_> = run
        .points
        .iter()
        .map(|(p, _)| project(*p, angle)[axis])
        .collect();
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if hi - lo > 16. * scale {
        return (coverage, false);
    }
    if side == 0 || side == 2 {
        (coverage.max(hi + scale), true)
    } else {
        (coverage.min(lo - scale), true)
    }
}

fn weak_border(
    side: usize,
    b: [f64; 4],
    angle: f64,
    c: &Capture,
    material: &Material,
    measure: [u32; 2],
    rgb: &[[u16; 3]],
) -> Option<f64> {
    let axis = usize::from(side >= 2);
    let other = if side < 2 { [2, 3] } else { [0, 1] };
    let (low, high) = (b[other[0]], b[other[1]]);
    let inward = if side == 0 || side == 2 { 1. } else { -1. };
    let step = (f64::from(c.size[0]) / 768.).max(f64::from(c.size[1]) / 512.);
    let distance = 2. * step;
    let extent = f64::from(c.size[axis]) * 0.15;
    let value = |uv| {
        let p = project(uv, -angle);
        if p.iter()
            .zip(c.size)
            .any(|(&v, n)| v < 0. || v >= f64::from(n))
            || material.label(p) != b'F'
        {
            return None;
        }
        let [x, y] = std::array::from_fn(|a| {
            (p[a] * f64::from(measure[a]) / f64::from(c.size[a])).floor() as usize
        });
        let codes = rgb[y * measure[0] as usize + x];
        Some(
            codes
                .iter()
                .map(|&v| f64::from(v.max(1)).log2())
                .sum::<f64>()
                / 3.,
        )
    };
    let per_block = (((high - low) * 0.9 / 8.
        * (f64::from(measure[0]) / f64::from(c.size[0]))
            .min(f64::from(measure[1]) / f64::from(c.size[1])))
    .floor() as usize)
        .clamp(1, 32);
    let mut rows = vec![];
    for offset in 4..((extent / step).floor() as usize).max(5) {
        let position = b[side] + inward * offset as f64 * step;
        let mut blocks = vec![];
        let mut slopes = vec![];
        for block in 0..8 {
            let mut deltas = vec![];
            let mut collars = vec![];
            for sample in 0..per_block {
                let along = low
                    + (high - low)
                        * (0.05
                            + 0.9 * (block as f64 + (sample as f64 + 0.5) / per_block as f64) / 8.);
                let values: Option<Vec<_>> = [
                    position - inward * 2. * distance,
                    position - inward * distance,
                    position + inward * distance,
                ]
                .into_iter()
                .map(|normal| {
                    value(if side < 2 {
                        [normal, along]
                    } else {
                        [along, normal]
                    })
                })
                .collect();
                if let Some(v) = values {
                    deltas.push(v[1] - v[2]);
                    collars.push(v[0] - v[1]);
                }
            }
            if deltas.len() >= 4 {
                blocks.push(deltas.iter().sum::<f64>() / deltas.len() as f64);
                slopes.push((collars.iter().sum::<f64>() / collars.len() as f64).abs());
            }
        }
        if blocks.len() < 6 {
            continue;
        }
        let jump = median(blocks.iter().copied());
        if jump >= 1. / 32.
            && blocks.iter().filter(|&&v| v > 0.).count() >= 6
            && jump > 2. * median(slopes)
        {
            rows.push((position, jump));
        }
    }
    let mut peaks: Vec<_> = rows
        .iter()
        .filter(|&&(p, j)| {
            !rows
                .iter()
                .any(|&(q, k)| (p - q).abs() <= 2. * step && k > j)
        })
        .copied()
        .collect();
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    let best = *peaks.first()?;
    if peaks
        .iter()
        .skip(1)
        .any(|(p, j)| (p - best.0).abs() > 8. * step && *j >= best.1 * 0.8)
    {
        None
    } else {
        Some(best.0)
    }
}

impl RollModel {
    /// Measurements must contain unmodified camera codes from the same native development,
    /// sampled at equal or higher resolution than the retained capture.
    pub fn areas(
        &self,
        index: usize,
        c: &Capture,
        measurement_size: [u32; 2],
        measurement: &[[u16; 3]],
    ) -> Result<Areas, Error> {
        if !self.owns(index, c) {
            return Err(invalid(
                "rectangle target differs from retained roll capture",
            ));
        }
        if measurement_size.iter().zip(c.size).any(|(&a, b)| a < b)
            || measurement.len() != measurement_size[0] as usize * measurement_size[1] as usize
        {
            return Err(invalid("invalid rectangle measurement raster"));
        }
        let Some(selection) = timed!("material_select", self.select(index, c)) else {
            return Ok(Areas::default());
        };
        timed!(
            "fine_geometry",
            fit_with_opening(
                c,
                selection,
                measurement_size,
                measurement,
                self.opening.as_ref(),
            )
        )
    }

    #[cfg(all(test, feature = "raw"))]
    pub(super) fn local_areas(
        &self,
        index: usize,
        c: &Capture,
        measurement_size: [u32; 2],
        measurement: &[[u16; 3]],
    ) -> Result<Areas, Error> {
        let selection = self
            .select(index, c)
            .ok_or(invalid("missing local material evidence"))?;
        fit(c, selection, measurement_size, measurement)
    }
}

#[cfg(test)]
pub(super) fn fit(
    c: &Capture,
    selection: Selection,
    measurement_size: [u32; 2],
    measurement: &[[u16; 3]],
) -> Result<Areas, Error> {
    fit_with_opening(c, selection, measurement_size, measurement, None)
}

fn fit_with_opening(
    c: &Capture,
    selection: Selection,
    measurement_size: [u32; 2],
    measurement: &[[u16; 3]],
    shared: Option<&super::opening::Shared>,
) -> Result<Areas, Error> {
    let observed = timed!(
        "holder_rays",
        runs(&selection, c.size, measurement_size, measurement)
    );
    let holder_angles: Vec<_> = observed
        .iter()
        .flatten()
        .filter(|r| r.points.iter().all(|(_, ok)| *ok) && r.angle.abs() <= 6.)
        .map(|r| r.angle)
        .collect();
    let opening_angle = if holder_angles.is_empty() {
        0.
    } else {
        median(holder_angles)
    };
    let angles: Vec<_> = (0..4)
        .filter(|&s| selection.supported[s])
        .filter_map(|s| {
            selection
                .family
                .as_ref()
                .and_then(|f| c.selected_line(f, s))
                .map(|l| l.physical_angle(s))
        })
        .collect();
    let angle = if angles.is_empty() {
        shared.map_or(opening_angle, |s| s.angle)
    } else {
        median(angles)
    };
    if !angle.is_finite() || angle.abs() > 6. {
        return Ok(Areas::default());
    }
    let scale = (f64::from(c.size[0]) / 768.).max(f64::from(c.size[1]) / 512.);
    let coverage = bounds(c.size, opening_angle, 2. * scale);
    let mut fitted: [(f64, bool); 4] = std::array::from_fn(|s| {
        opening_side(s, observed[s].as_ref(), opening_angle, scale, coverage[s])
    });
    let initial = fitted;
    // Only independently supported cross-sides associate corner rays; no circular completion.
    for side in 0..4 {
        if initial[side].1 {
            continue;
        }
        let other = if side < 2 { [2, 3] } else { [0, 1] };
        if !other.iter().all(|&s| initial[s].1) {
            continue;
        }
        if let Some(run) = &observed[side] {
            let axis = usize::from(side < 2);
            let mut associated = run.clone();
            associated.points.retain(|(p, _)| {
                let v = project(*p, opening_angle)[axis];
                initial[other[0]].0 <= v && v <= initial[other[1]].0
            });
            let candidate = opening_side(
                side,
                Some(&associated),
                opening_angle,
                scale,
                coverage[side],
            );
            if candidate.1 {
                fitted[side] = candidate;
            }
        }
    }
    let Some(local_opening) = polygon(fitted.map(|v| v.0), opening_angle) else {
        return Ok(Areas::default());
    };
    let (opening, exception) = shared.map_or((Some(local_opening), false), |s| {
        let own_rails = std::array::from_fn(|side| {
            observed[side].as_ref().and_then(|run| {
                super::opening::rail(c.size, s.angle, side, run.points.iter().map(|&(p, _)| p))
            })
        });
        timed!(
            "shared_constrain",
            s.constrain(c.size, local_opening, fitted.map(|v| v.1), own_rails)
        )
    });
    let Some(opening) = opening else {
        return Ok(Areas::default());
    };
    let mut b = bounds(c.size, angle, 2. * scale);
    let projected = opening.map(|p| project(p, angle));
    for (side, bound) in b.iter_mut().enumerate() {
        let axis = usize::from(side >= 2);
        if side == 0 || side == 2 {
            *bound = bound.max(
                projected
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::INFINITY, f64::min),
            );
        } else {
            *bound = bound.min(
                projected
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
        }
    }
    let mut constraints = vec![];
    let mut sides = [None; 4];
    for (side, supported) in selection.supported.iter().enumerate() {
        if !supported {
            continue;
        }
        let l = c
            .selected_line(selection.family.as_ref().unwrap(), side)
            .unwrap();
        let axis = usize::from(side < 2);
        let values: Vec<_> = l
            .spans()
            .into_iter()
            .flat_map(|[lo, hi]| [lo, (lo + hi) / 2., hi])
            .map(|t| {
                let along = f64::from(c.size[axis] - 1) * t + 0.5;
                let cross = l.cross(side, c.size, along);
                project(
                    if side < 2 {
                        [cross, along]
                    } else {
                        [along, cross]
                    },
                    angle,
                )[1 - axis]
            })
            .collect();
        let bound = median(values);
        sides[side] = Some(bound);
        b[side] = if side == 0 || side == 2 {
            b[side].max(bound)
        } else {
            b[side].min(bound)
        };
        let [a, bb, cc] = l.equation(side, c.size);
        let a = a * f64::from(c.size[0]) / f64::from(c.area.width);
        let bb = bb * f64::from(c.size[1]) / f64::from(c.area.height);
        let cc = cc + a * f64::from(c.area.x) + bb * f64::from(c.area.y);
        let sign = if side == 0 || side == 2 { 1. } else { -1. };
        constraints.push([a * sign, bb * sign, cc * sign]);
    }
    let profile_bounds = b;
    let mut weak = [false; 4];
    for (side, supported) in selection.supported.iter().enumerate() {
        if !supported
            && selection.family.is_none()
            && let Some(bound) = weak_border(
                side,
                profile_bounds,
                angle,
                c,
                &selection.material,
                measurement_size,
                measurement,
            )
        {
            b[side] = bound;
            sides[side] = Some(bound);
            weak[side] = true;
        }
    }
    let result = Areas {
        opening: Some(opening.map(|p| c.source_point(p))),
        picture: polygon(b, angle).map(|q| q.map(|p| c.source_point(p))),
        material: Some(selection.material),
        picture_constraints: constraints,
        opening_observed: if shared.is_some() {
            [false; 4]
        } else {
            fitted.map(|v| v.1)
        },
        opening_shared: shared.is_some(),
        opening_exception: exception,
        picture_geometry: Some(PictureGeometry {
            size: c.size,
            angle,
            sides,
        }),
        picture_observed: selection.supported,
        picture_weak: weak,
        opening_edited: false,
        picture_edited: false,
    };
    result.validate(c.area)?;
    Ok(result)
}
