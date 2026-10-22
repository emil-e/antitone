use super::lines::{Family, Line};
use super::*;

#[path = "framing/mod.rs"]
pub(super) mod framing;
use crate::region_timing::timed;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone)]
struct Base {
    mean: [f64; 3],
    spread: [f64; 3],
}

pub struct RollModel {
    area: SourceRegion,
    size: [u32; 2],
    xs: Vec<u32>,
    ys: Vec<u32>,
    grids: Vec<Vec<[u16; 3]>>,
    base: Option<Base>,
    tail: Option<[f64; 3]>,
    upper: Option<[f64; 3]>,
    dark: Vec<Vec<bool>>,
    atlas: Vec<Vec<[Option<f64>; 2]>>,
    positions: Vec<[Vec<(f64, f64, f64)>; 4]>,
    pub(super) opening: Option<super::opening::Shared>,
}

pub(super) struct Selection {
    pub material: Material,
    pub upper: Option<[f64; 3]>,
    pub family: Option<Family>,
    pub supported: [bool; 4],
}

fn logs(v: [u16; 3]) -> Option<[f64; 3]> {
    v.iter()
        .all(|&v| v > 1 && v < 65535)
        .then(|| v.map(|v| f64::from(v).log2()))
}

fn affinity(v: [u16; 3], base: &Base) -> Option<f64> {
    let v = logs(v)?;
    Some(
        (-(0..3)
            .map(|c| ((v[c] - base.mean[c]) / base.spread[c]).powi(2))
            .sum::<f64>()
            / 6.)
            .exp(),
    )
}

fn dark_affinity(v: [u16; 3], tail: Option<[f64; 3]>) -> Option<f64> {
    let t = tail?;
    let exponent = (0..3)
        .map(|c| (f64::from(v[c].max(1)) / t[c]).log2())
        .fold(f64::NEG_INFINITY, f64::max)
        / 0.25;
    Some(if exponent >= 0. {
        let v = (-exponent).exp();
        v / (1. + v)
    } else {
        1. / (1. + exponent.exp())
    })
}

fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let x = i % w;
    let y = i / w;
    [
        (x > 0).then(|| i - 1),
        (x + 1 < w).then_some(i + 1),
        (y > 0).then(|| i - w),
        (y + 1 < h).then_some(i + w),
    ]
    .into_iter()
    .flatten()
}

fn connected(allowed: &[bool], w: usize, h: usize) -> BTreeSet<usize> {
    let seeds: Vec<_> = (0..w * h)
        .filter(|&i| allowed[i] && (i % w == 0 || i % w == w - 1 || i / w == 0 || i / w == h - 1))
        .collect();
    let mut found: BTreeSet<_> = seeds.iter().copied().collect();
    let mut queue: VecDeque<_> = seeds.into();

    while let Some(i) = queue.pop_front() {
        for j in neighbours(i, w, h) {
            if allowed[j] && found.insert(j) {
                queue.push_back(j);
            }
        }
    }
    found
}

fn fit_base(grids: &[Vec<[u16; 3]>]) -> Option<Base> {
    let mut hist: BTreeMap<[i32; 3], Vec<f64>> = BTreeMap::new();
    let mut selected = vec![];
    for (n, grid) in grids.iter().enumerate() {
        let valid: Vec<_> = grid.iter().filter_map(|&v| logs(v)).collect();
        let cut = quantile(valid.iter().map(|v| v.iter().sum::<f64>() / 3.), 0.75);
        let bright: Vec<_> = valid
            .into_iter()
            .filter(|v| cut.is_some_and(|cut| v.iter().sum::<f64>() / 3. >= cut))
            .collect();
        for v in &bright {
            let key = v.map(|v| (v / 0.25).floor() as i32);
            hist.entry(key).or_insert_with(|| vec![0.; grids.len()])[n] += 1. / grid.len() as f64;
        }
        selected.extend(bright);
    }

    let deltas: Vec<_> = (-1..=1)
        .flat_map(|a| (-1..=1).flat_map(move |b| (-1..=1).map(move |c| [a, b, c])))
        .collect();
    let mut peaks = vec![];
    for (&key, votes) in &hist {
        let mass = votes.iter().sum::<f64>();
        let nearby: Vec<_> = deltas
            .iter()
            .filter_map(|d| hist.get(&std::array::from_fn(|c| key[c] + d[c])))
            .collect();
        if nearby.iter().any(|v| v.iter().sum::<f64>() > mass) {
            continue;
        }
        let counts: Vec<_> = (0..grids.len())
            .map(|n| nearby.iter().map(|v| v[n]).sum::<f64>())
            .collect();
        let support = counts.iter().filter(|&&v| v > 0.).count();
        let population = counts.iter().sum::<f64>() / grids.len() as f64;
        if support >= 2.max(grids.len().div_ceil(2)) && population >= 0.005 {
            peaks.push((key, population));
        }
    }

    peaks.sort_by(|(a, ap), (b, bp)| {
        b.iter()
            .sum::<i32>()
            .cmp(&a.iter().sum())
            .then_with(|| bp.total_cmp(ap))
            .then_with(|| a.cmp(b))
    });

    let key = peaks.first()?.0;
    let values: Vec<_> = selected
        .into_iter()
        .filter(|v| (0..3).all(|c| ((v[c] / 0.25).floor() as i32 - key[c]).abs() <= 1))
        .collect();
    let mean = std::array::from_fn(|c| quantile(values.iter().map(|v| v[c]), 0.5).unwrap());
    let spread = std::array::from_fn(|c| {
        (1.4826 * quantile(values.iter().map(|v| (v[c] - mean[c]).abs()), 0.5).unwrap()).max(0.125)
    });
    Some(Base { mean, spread })
}

fn line_position(capture: &Capture, side: usize, line: &Line) -> f64 {
    let axis = usize::from(side >= 2);
    let origin = if axis == 0 {
        capture.area.x
    } else {
        capture.area.y
    };
    let extent = if axis == 0 {
        capture.area.width
    } else {
        capture.area.height
    };
    (f64::from(origin)
        + (line.midpoint_pixel + 0.5) * f64::from(extent) / f64::from(capture.size[axis]))
        / (f64::from(capture.area.width.max(capture.area.height)) / 768.)
}

impl RollModel {
    /// Pool the normal fine-measurement interiors before freezing roll support.
    /// Authored geometry and evidence-backed photo exceptions remain independent.
    pub fn refine_areas(&self, areas: &mut [Areas]) -> Result<(), Error> {
        if areas.len() != self.grids.len() {
            return Err(invalid("unordered shared opening refinement"));
        }
        for a in areas.iter() {
            a.validate(self.area)?;
        }
        if let Some(shared) = &self.opening {
            shared.refine(self.size, self.area, areas);
        }
        for a in areas.iter() {
            a.validate(self.area)?;
        }
        Ok(())
    }

    pub fn fit(captures: &[Capture]) -> Result<Self, Error> {
        Self::fit_refs(&captures.iter().collect::<Vec<_>>())
    }

    pub fn fit_refs(captures: &[&Capture]) -> Result<Self, Error> {
        let first = captures.first().ok_or(invalid("empty region roll"))?;
        let size = first.size;
        if captures
            .iter()
            .any(|v| v.size != size || v.area != first.area)
        {
            return Err(invalid(
                "automatic roll areas require matching source geometry",
            ));
        }
        let xs = centres(size[0], size[0].min(96));
        let ys = centres(size[1], size[1].min(64));
        let w = xs.len();
        let h = ys.len();
        let grids: Vec<Vec<_>> = captures
            .iter()
            .map(|v| {
                ys.iter()
                    .flat_map(|&y| {
                        xs.iter()
                            .map(move |&x| v.rgb[y as usize * size[0] as usize + x as usize])
                    })
                    .collect()
            })
            .collect();
        let base = timed!("base_fit", fit_base(&grids));
        let central: Vec<_> = grids
            .iter()
            .flat_map(|g| {
                g.iter()
                    .enumerate()
                    .filter(|(i, rgb)| {
                        let x = f64::from(xs[i % w]);
                        let y = f64::from(ys[i / w]);
                        x >= 0.25 * f64::from(size[0])
                            && x < 0.75 * f64::from(size[0])
                            && y >= 0.25 * f64::from(size[1])
                            && y < 0.75 * f64::from(size[1])
                            && logs(**rgb).is_some()
                    })
                    .map(|(_, v)| *v)
            })
            .collect();
        let tail = (!central.is_empty()).then(|| {
            std::array::from_fn(|c| {
                quantile(central.iter().map(|v| f64::from(v[c])), 0.005).unwrap()
            })
        });
        let mut exterior = vec![];
        if let Some(base) = &base {
            for g in &grids {
                for (i, rgb) in g.iter().enumerate() {
                    if (i % w == 0 || i % w == w - 1 || i / w == 0 || i / w == h - 1)
                        && (0..3).all(|c| f64::from(rgb[c]) <= base.mean[c].exp2() / 16.)
                    {
                        exterior.push(*rgb);
                    }
                }
            }
        }
        let upper = (!exterior.is_empty()).then(|| {
            std::array::from_fn(|c| {
                2. * quantile(exterior.iter().map(|v| f64::from(v[c])), 0.99).unwrap() + 2.
            })
        });
        let dark = grids
            .iter()
            .map(|g| {
                g.iter()
                    .map(|v| upper.is_some_and(|u| (0..3).all(|c| f64::from(v[c]) <= u[c])))
                    .collect()
            })
            .collect();
        let atlas = grids
            .iter()
            .map(|g| {
                g.iter()
                    .map(|&v| {
                        [
                            base.as_ref().and_then(|b| affinity(v, b)),
                            dark_affinity(v, tail),
                        ]
                    })
                    .collect()
            })
            .collect();
        let positions = captures
            .iter()
            .map(|v| {
                std::array::from_fn(|s| {
                    let mut votes: Vec<_> = v.group.lines[s]
                        .iter()
                        .filter(|l| l.fraction() >= 0.1)
                        .map(|l| {
                            (
                                line_position(v, s, l),
                                l.angle_degrees,
                                l.fraction() * l.gradient_ratio.ln_1p(),
                            )
                        })
                        .collect();
                    let total = votes.iter().map(|v| v.2).sum::<f64>();
                    if total > 0. {
                        for v in &mut votes {
                            v.2 /= total;
                        }
                    } else {
                        votes.clear();
                    }
                    votes
                })
            })
            .collect();
        let opening = timed!(
            "shared_holder_fit",
            super::opening::Shared::fit(captures, upper)
        );
        Ok(Self {
            area: first.area,
            size,
            xs,
            ys,
            grids,
            base,
            tail,
            upper,
            dark,
            atlas,
            positions,
            opening,
        })
    }

    pub(super) fn owns(&self, index: usize, c: &Capture) -> bool {
        self.area == c.area
            && self.size == c.size
            && self.grids.get(index).is_some_and(|grid| {
                self.ys
                    .iter()
                    .flat_map(|&y| {
                        self.xs
                            .iter()
                            .map(move |&x| c.rgb[y as usize * c.size[0] as usize + x as usize])
                    })
                    .eq(grid.iter().copied())
            })
    }

    fn partition(&self, index: usize) -> Option<Material> {
        if self.grids.len() < 2 {
            return None;
        }
        let base = self.base.as_ref()?;
        let w = self.xs.len();
        let h = self.ys.len();
        let peers = self.grids.len() - 1;
        let allowed: Vec<_> = (0..w * h)
            .map(|i| {
                self.dark[index][i]
                    && self
                        .dark
                        .iter()
                        .enumerate()
                        .filter(|(n, v)| *n != index && v[i])
                        .count()
                        >= (0.75 * peers as f64).ceil() as usize
            })
            .collect();
        let holder = connected(&allowed, w, h);
        let mut labels = vec![b'F'; w * h];
        for &i in &holder {
            labels[i] = b'H';
        }
        let mut bright = BTreeSet::new();
        for (i, rgb) in self.grids[index].iter().enumerate() {
            if holder.contains(&i) || rgb.iter().any(|&v| v <= 1) {
                continue;
            }
            let d: [f64; 3] = std::array::from_fn(|c| f64::from(rgb[c]).log2() - base.mean[c]);
            if (0..3).all(|c| d[c] >= -3. * base.spread[c])
                && (0..3).any(|c| d[c] > (3. * base.spread[c]).max(0.5))
            {
                bright.insert(i);
            }
        }
        let mut pending = bright.clone();
        while let Some(start) = pending.pop_first() {
            let mut part = BTreeSet::from([start]);
            let mut queue = VecDeque::from([start]);
            while let Some(i) = queue.pop_front() {
                for j in neighbours(i, w, h) {
                    if pending.remove(&j) {
                        part.insert(j);
                        queue.push_back(j);
                    }
                }
            }
            let adjacent: BTreeSet<_> = part
                .iter()
                .flat_map(|&i| neighbours(i, w, h))
                .filter(|i| !part.contains(i))
                .collect();
            let exterior = part
                .iter()
                .any(|i| i % w == 0 || i % w == w - 1 || i / w == 0 || i / w == h - 1)
                || adjacent.iter().any(|i| holder.contains(i));
            if exterior
                && adjacent
                    .iter()
                    .any(|i| !holder.contains(i) && !bright.contains(i))
            {
                for i in part {
                    labels[i] = b'L';
                }
            }
        }
        if !labels
            .iter()
            .enumerate()
            .any(|(i, &v)| v == b'F' && self.atlas[index][i][0].is_some_and(|v| v >= 0.5))
        {
            return None;
        }
        Some(Material {
            size: self.size,
            xs: self.xs.clone(),
            ys: self.ys.clone(),
            labels: String::from_utf8(labels).expect("ASCII material labels"),
        })
    }

    fn appearance(&self, index: usize, p: [f64; 2], rgb: [u16; 3]) -> Option<f64> {
        let i =
            Material::nearest(&self.ys, p[1]) * self.xs.len() + Material::nearest(&self.xs, p[0]);
        let own = [
            self.base.as_ref().and_then(|b| affinity(rgb, b)),
            dark_affinity(rgb, self.tail),
        ];
        let peers = self.grids.len() - 1;
        if peers == 0 {
            return None;
        }
        (0..2)
            .filter_map(|role| {
                let v = own[role]?;
                let vals: Vec<_> = self
                    .atlas
                    .iter()
                    .enumerate()
                    .filter(|(n, _)| *n != index)
                    .filter_map(|(_, a)| a[i][role])
                    .collect();
                (!vals.is_empty()).then(|| v * vals.iter().sum::<f64>() / peers as f64)
            })
            .max_by(f64::total_cmp)
    }

    fn side_prior(
        &self,
        index: usize,
        capture: &Capture,
        family: &Family,
        side: usize,
        line: &Line,
    ) -> f64 {
        let axis = usize::from(side < 2);
        let inward = if side == 0 || side == 2 { 1. } else { -1. };
        let mut values = vec![];
        let mut seen = BTreeSet::new();
        for along in indices(capture.size[axis], 32) {
            if !line.observed(f64::from(along) / f64::from(capture.size[axis] - 1)) {
                continue;
            }
            let a = f64::from(along) + 0.5;
            let cross = line.cross(side, capture.size, a);
            let p = if side < 2 { [cross, a] } else { [a, cross] };
            if !capture.within(p, side, family) {
                continue;
            }
            let mut observations = vec![];
            for d in [2., 4., 8.] {
                let c = (cross - inward * d).floor() as i32;
                let p = if side < 2 {
                    [c, along as i32]
                } else {
                    [along as i32, c]
                };
                if !seen.insert(p) {
                    continue;
                }
                if let Some(rgb) = capture.rgb(p)
                    && let Some(v) =
                        self.appearance(index, [f64::from(p[0]) + 0.5, f64::from(p[1]) + 0.5], rgb)
                {
                    observations.push(v);
                }
            }
            if !observations.is_empty() {
                values.push(observations.iter().sum::<f64>() / observations.len() as f64);
            }
        }
        let appearance = (values.iter().sum::<f64>() + 1.) / (values.len() + 2) as f64;
        let position = line_position(capture, side, line);
        let support = self
            .positions
            .iter()
            .enumerate()
            .filter(|(n, _)| *n != index)
            .map(|(_, p)| {
                p[side]
                    .iter()
                    .map(|&(v, a, w)| {
                        w * (-0.5 * ((position - v) / 8.).powi(2)
                            - 0.5 * ((line.angle_degrees - a) / 2.).powi(2))
                        .exp()
                    })
                    .sum::<f64>()
            })
            .sum::<f64>();
        let extent = f64::from(if side < 2 {
            capture.area.width
        } else {
            capture.area.height
        }) / (f64::from(capture.area.width.max(capture.area.height)) / 768.);
        let uniform = (std::f64::consts::TAU.sqrt() * 8. / (extent / 4.)).min(1.);
        let layout = (support + uniform) / self.grids.len() as f64;
        4. * (appearance.ln() + layout.ln())
    }

    pub(super) fn select(&self, index: usize, capture: &Capture) -> Option<Selection> {
        let material = timed!("material_partition", self.partition(index))?;
        let upper = self.upper;
        let mut rows = vec![];
        for family in &capture.group.families {
            let mut supported = [false; 4];
            let mut denser = [false; 4];
            let mut prior = 0.;
            for (s, support) in supported.iter_mut().enumerate() {
                let Some(line) = capture.selected_line(family, s) else {
                    continue;
                };
                let (t, d, film) = side_evidence(capture, &material, family, s, line);
                denser[s] = d;
                *support = line.fraction() >= 0.1 && (t || d) && film >= 0.75;
                if *support {
                    prior += self.side_prior(index, capture, family, s, line);
                }
            }
            let count = supported.iter().filter(|&&v| v).count();
            if count < 2 {
                continue;
            }
            rows.push((family.clone(), supported, denser, prior / count as f64));
        }
        rows.sort_by(|a, b| {
            let unexplained = |v: &(Family, [bool; 4], [bool; 4], f64)| {
                v.0.indices.iter().flatten().count() - v.1.iter().filter(|&&v| v).count()
            };
            unexplained(a)
                .cmp(&unexplained(b))
                .then_with(|| b.3.total_cmp(&a.3))
                .then_with(|| b.0.score.total_cmp(&a.0.score))
        });
        if let Some(leader) = rows.first()
            && !leader.0.complete
            && leader.1.iter().all(|&v| v)
        {
            let matched = rows
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, v)| {
                    v.0.complete
                        && v.1.iter().all(|&v| v)
                        && (0..4).all(|s| {
                            let a = capture.selected_line(&leader.0, s).unwrap();
                            let b = capture.selected_line(&v.0, s).unwrap();
                            let common: Vec<_> = a
                                .spans()
                                .iter()
                                .flat_map(|[lo, hi]| {
                                    b.spans().into_iter().filter_map(move |[low, high]| {
                                        let l = lo.max(low);
                                        let h = hi.min(high);
                                        (l < h).then_some([l, h])
                                    })
                                })
                                .collect();
                            if common.iter().map(|[a, b]| b - a).sum::<f64>() < 0.1 {
                                return false;
                            }
                            let mut shared = a.clone();
                            shared.span_intervals = Some(common);
                            let radius = 2.
                                + 0.125f64.to_radians().tan()
                                    * f64::from(capture.size[usize::from(s < 2)])
                                    / 2.;
                            let same = a.midpoint_pixel == b.midpoint_pixel
                                && a.angle_degrees == b.angle_degrees;
                            (same || leader.2[s] && v.2[s])
                                && (a.angle_degrees - b.angle_degrees).abs() <= 0.75
                                && b.distance(&shared, s, capture.size) <= radius
                        })
                })
                .map(|(i, _)| i);
            if let Some(i) = matched {
                let row = rows.remove(i);
                rows.insert(0, row);
            }
        }
        let (family, supported) = rows
            .into_iter()
            .next()
            .map_or((None, [false; 4]), |(f, s, _, _)| (Some(f), s));
        Some(Selection {
            material,
            upper,
            family,
            supported,
        })
    }
}

fn indices(extent: u32, count: u32) -> BTreeSet<u32> {
    (0..count)
        .map(|i| {
            (f64::from(extent - 1) * (0.1 + 0.8 * f64::from(i) / f64::from(count - 1)) + 0.5)
                .floor() as u32
        })
        .collect()
}

fn side_evidence(c: &Capture, m: &Material, f: &Family, s: usize, l: &Line) -> (bool, bool, f64) {
    let axis = usize::from(s < 2);
    let inward = if s == 0 || s == 2 { 1. } else { -1. };
    let mut opportunities = 0;
    let mut differences = vec![];
    let mut deltas = vec![];
    let mut film = vec![];
    for index in indices(c.size[axis], 96) {
        let frac = f64::from(index) / f64::from(c.size[axis] - 1);
        if !l.observed(frac) {
            continue;
        }
        opportunities += 1;
        let along = f64::from(index) + 0.5;
        let cross = l.cross(s, c.size, along);
        let p = if s < 2 {
            [cross, along]
        } else {
            [along, cross]
        };
        if !c.within(p, s, f) {
            continue;
        }
        let mut textures = [vec![], vec![]];
        if [-1., 0., 1.]
            .iter()
            .all(|d| l.observed((f64::from(index) + d) / f64::from(c.size[axis] - 1)))
        {
            for d in [2., 4., 8.] {
                let mut pairs = vec![];
                for sign in [1., -1.] {
                    let normal = (cross + sign * inward * d).floor() as i32;
                    let pair: Option<Vec<_>> = [-1, 1]
                        .into_iter()
                        .map(|delta| {
                            c.rgb(if s < 2 {
                                [normal, index as i32 + delta]
                            } else {
                                [index as i32 + delta, normal]
                            })
                        })
                        .collect();
                    if let Some(pair) = pair {
                        pairs.push(pair);
                    }
                }
                if pairs.len() != 2 {
                    continue;
                }
                let channels: Vec<_> = (0..3)
                    .filter(|&ch| pairs.iter().flatten().all(|v| v[ch] > 0 && v[ch] < 65535))
                    .collect();
                if channels.len() < 2 {
                    continue;
                }
                for (sign, pair) in pairs.iter().enumerate() {
                    let magnitude = channels
                        .iter()
                        .map(|&ch| ((f64::from(pair[0][ch]) + f64::from(pair[1][ch])) / 2.).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    textures[sign].push(
                        channels
                            .iter()
                            .map(|&ch| (f64::from(pair[0][ch]) - f64::from(pair[1][ch])).powi(2))
                            .sum::<f64>()
                            .sqrt()
                            / magnitude,
                    );
                }
            }
            if !textures[0].is_empty() {
                differences.push(
                    textures[0].iter().sum::<f64>() / textures[0].len() as f64
                        - textures[1].iter().sum::<f64>() / textures[1].len() as f64,
                );
            }
        }
        let points = [1., -1.].map(|sign| {
            let n = (cross + sign * inward).floor() as i32;
            if s < 2 {
                [n, index as i32]
            } else {
                [index as i32, n]
            }
        });
        if let (Some(a), Some(b)) = (c.rgb(points[0]), c.rgb(points[1])) {
            deltas.push(std::array::from_fn::<_, 3, _>(|ch| {
                (f64::from(a[ch]) - f64::from(b[ch])) / 65535.
            }));
        }
    }
    // Material-role opportunities use 32 own-source positions, independently of texture/transition.
    for index in indices(c.size[axis], 32) {
        if !l.observed(f64::from(index) / f64::from(c.size[axis] - 1)) {
            continue;
        }
        let along = f64::from(index) + 0.5;
        let cross = l.cross(s, c.size, along);
        if !c.within(
            if s < 2 {
                [cross, along]
            } else {
                [along, cross]
            },
            s,
            f,
        ) {
            continue;
        }
        for d in [4., 8.] {
            let n = (cross + inward * d).floor() as i32;
            let p = if s < 2 {
                [n, index as i32]
            } else {
                [index as i32, n]
            };
            if c.rgb(p).is_some() {
                film.push(m.label([f64::from(p[0]) + 0.5, f64::from(p[1]) + 0.5]) == b'F');
            }
        }
    }
    let texture = differences.len() as f64 >= 0.75 * opportunities as f64
        && quantile(differences, 0.25).is_some_and(|v| v > 0.);
    let mean: [f64; 3] = std::array::from_fn(|ch| {
        deltas.iter().map(|v| v[ch]).sum::<f64>() / deltas.len().max(1) as f64
    });
    let length = mean.iter().map(|v| v * v).sum::<f64>().sqrt();
    let projection = quantile(
        deltas
            .iter()
            .map(|v| v.iter().zip(mean).map(|(a, b)| a * b).sum::<f64>() / length),
        0.25,
    );
    let denser = length > 0.
        && deltas.len() as f64 >= 0.75 * opportunities as f64
        && projection.is_some_and(|v| v > 0.)
        && mean.iter().all(|&v| v <= 0.)
        && mean.iter().any(|&v| v < 0.);
    let fraction = film.iter().filter(|&&v| v).count() as f64 / film.len().max(1) as f64;
    (texture, denser, fraction)
}
