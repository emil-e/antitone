use super::*;
use std::collections::{BTreeMap, BTreeSet};
pub(super) const SIDES: [&str; 4] = ["left", "right", "top", "bottom"];

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Surface {
    pub median_camera_rgb: [f64; 3],
}
#[derive(Clone, Debug, Deserialize)]
pub(super) struct Line {
    pub midpoint_pixel: f64,
    pub angle_degrees: f64,
    pub lower_quartile_gradient_score: f64,
    pub local_gradient_background: Option<f64>,
    pub angle_limit_reached: bool,
    pub outside_surface: Option<Surface>,
    pub span_fraction: [f64; 2],
    #[serde(default)]
    pub span_intervals: Option<Vec<[f64; 2]>>,
    pub partial_span: bool,
    #[serde(default)]
    pub gradient_ratio: f64,
    #[serde(default)]
    pub rank: usize,
}
impl Line {
    pub fn spans(&self) -> Vec<[f64; 2]> {
        self.span_intervals
            .clone()
            .unwrap_or_else(|| vec![self.span_fraction])
    }
    pub fn fraction(&self) -> f64 {
        self.spans().iter().map(|[a, b]| b - a).sum()
    }
    pub fn observed(&self, fraction: f64) -> bool {
        self.spans()
            .iter()
            .any(|[a, b]| *a <= fraction && fraction <= *b)
    }
    pub fn physical_angle(&self, side: usize) -> f64 {
        self.angle_degrees * if side < 2 { -1. } else { 1. }
    }
    pub fn equation(&self, side: usize, size: [u32; 2]) -> [f64; 3] {
        let slope = self.angle_degrees.to_radians().tan();
        let p = self.midpoint_pixel + 0.5;
        if side < 2 {
            [1., -slope, p - slope * f64::from(size[1]) / 2.]
        } else {
            [-slope, 1., p - slope * f64::from(size[0]) / 2.]
        }
    }
    pub fn cross(&self, side: usize, size: [u32; 2], along: f64) -> f64 {
        let [a, b, c] = self.equation(side, size);
        if side < 2 {
            (c - b * along) / a
        } else {
            (c - a * along) / b
        }
    }
    pub fn distance(&self, other: &Self, side: usize, size: [u32; 2]) -> f64 {
        let [a, b, c] = self.equation(side, size);
        let axis = usize::from(side < 2);
        other
            .spans()
            .iter()
            .flatten()
            .map(|&t| {
                let along = f64::from(size[axis] - 1) * t + 0.5;
                let cross = other.cross(side, size, along);
                let [x, y] = if side < 2 {
                    [cross, along]
                } else {
                    [along, cross]
                };
                (a * x + b * y - c).abs() / a.hypot(b)
            })
            .fold(0., f64::max)
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Evidence {
    #[serde(alias = "oriented_line_candidates")]
    pub(super) lines: BTreeMap<String, Vec<Line>>,
    #[serde(alias = "segment_line_candidates")]
    pub(super) segments: BTreeMap<String, Vec<Line>>,
}
impl Evidence {
    #[cfg(feature = "raw")]
    pub fn observe(size: [u32; 2], rgb: &[[u16; 3]]) -> Result<Self, Error> {
        unsafe extern "C" {
            fn antitone_scan_region_lines(rgb: *const u16, w: u32, h: u32)
            -> *mut std::ffi::c_char;
            fn antitone_scan_region_lines_free(data: *mut std::ffi::c_char);
        }
        if size.iter().any(|&v| !(16..=768).contains(&v))
            || rgb.len() != size[0] as usize * size[1] as usize
        {
            return Err(invalid("invalid region camera raster"));
        }
        // Native code reads only this immutable, length-checked RGB16 view and returns owned JSON.
        let data = unsafe { antitone_scan_region_lines(rgb.as_ptr().cast(), size[0], size[1]) };
        if data.is_null() {
            return Err(invalid("cannot observe scan-region lines"));
        }
        let result = serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(data) }.to_bytes())
            .map_err(|_| invalid("invalid native region evidence"));
        unsafe { antitone_scan_region_lines_free(data) };
        result
    }
}
#[derive(Clone, Debug)]
pub(super) struct Family {
    pub indices: [Option<usize>; 4],
    pub score: f64,
    pub complete: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Group {
    pub lines: [Vec<Line>; 4],
    pub families: Vec<Family>,
}

pub struct Capture {
    pub area: SourceRegion,
    pub size: [u32; 2],
    pub rgb: Vec<[u16; 3]>,
    pub(super) group: Group,
}
impl Capture {
    pub fn new(
        area: SourceRegion,
        size: [u32; 2],
        rgb: Vec<[u16; 3]>,
        evidence: Evidence,
    ) -> Result<Self, Error> {
        if size.iter().any(|&v| !(16..=768).contains(&v))
            || rgb.len() != size[0] as usize * size[1] as usize
            || area.width < 16
            || area.height < 16
        {
            return Err(invalid("invalid region camera raster"));
        }
        area.validate()?;
        let group = group(&evidence, size)?;
        Ok(Self {
            area,
            size,
            rgb,
            group,
        })
    }
    pub(super) fn rgb(&self, p: [i32; 2]) -> Option<[u16; 3]> {
        if p.iter()
            .zip(self.size)
            .any(|(&v, n)| v < 0 || v >= n as i32)
        {
            None
        } else {
            Some(self.rgb[p[1] as usize * self.size[0] as usize + p[0] as usize])
        }
    }
    pub(super) fn within(&self, p: [f64; 2], side: usize, family: &Family) -> bool {
        family.indices.iter().enumerate().all(|(s, i)| {
            s == side
                || i.is_none_or(|i| {
                    let [a, b, c] = self.group.lines[s][i].equation(s, self.size);
                    let delta = a * p[0] + b * p[1] - c;
                    if s == 0 || s == 2 {
                        delta >= 0.
                    } else {
                        delta <= 0.
                    }
                })
        })
    }
    pub(super) fn selected_line(&self, family: &Family, side: usize) -> Option<&Line> {
        family.indices[side].map(|i| &self.group.lines[side][i])
    }
    pub(super) fn source_point(&self, p: [f64; 2]) -> [f64; 2] {
        [
            f64::from(self.area.x) + p[0] * f64::from(self.area.width) / f64::from(self.size[0]),
            f64::from(self.area.y) + p[1] * f64::from(self.area.height) / f64::from(self.size[1]),
        ]
    }
}
fn group(evidence: &Evidence, size: [u32; 2]) -> Result<Group, Error> {
    let mut lines: [Vec<Line>; 4] = std::array::from_fn(|_| vec![]);
    for s in 0..4 {
        let rows = evidence
            .lines
            .get(SIDES[s])
            .ok_or(invalid("missing region evidence side"))?;
        if rows.len() > 5 {
            return Err(invalid("too many region line candidates"));
        }
        for (rank, l) in rows.iter().enumerate() {
            if let Some(background) = l.local_gradient_background {
                let mut l = l.clone();
                l.rank = rank;
                l.gradient_ratio = l.lower_quartile_gradient_score / background.max(1. / 65535.);
                if !l.angle_limit_reached && l.gradient_ratio >= 4. {
                    lines[s].push(l);
                }
            }
        }
    }
    let long: Vec<_> = lines
        .iter()
        .enumerate()
        .flat_map(|(s, ls)| ls.iter().map(move |l| (s, l.physical_angle(s))))
        .collect();
    let mut guides = vec![];
    for (i, &(s, a)) in long.iter().enumerate() {
        for &(t, b) in &long[i + 1..] {
            if s != t && (a - b).abs() <= 1.5 {
                guides.push((a + b) / 2.);
            }
        }
    }
    for s in 0..4 {
        let mut admitted: Vec<_> = evidence
            .segments
            .get(SIDES[s])
            .ok_or(invalid("missing region segment evidence"))?
            .iter()
            .filter_map(|l| {
                let bg = l.local_gradient_background?;
                let mut l = l.clone();
                l.gradient_ratio = l.lower_quartile_gradient_score / bg.max(1. / 65535.);
                (!l.angle_limit_reached
                    && l.gradient_ratio >= 4.
                    && guides
                        .iter()
                        .any(|a| (l.physical_angle(s) - a).abs() <= 0.75))
                .then_some(l)
            })
            .collect();
        admitted.sort_by(|a, b| b.gradient_ratio.total_cmp(&a.gradient_ratio));
        let mut merged: Vec<Line> = vec![];
        for mut l in admitted {
            if let Some(prior) = merged.iter_mut().find(|p| {
                (l.angle_degrees - p.angle_degrees).abs() <= 0.75 && p.distance(&l, s, size) <= 2.
            }) {
                let mut spans = prior.spans();
                spans.extend(l.spans());
                spans.sort_by(|a, b| a[0].total_cmp(&b[0]));
                let mut union: Vec<[f64; 2]> = vec![];
                for [a, b] in spans {
                    if let Some(last) = union.last_mut().filter(|v| a <= v[1]) {
                        last[1] = last[1].max(b);
                    } else {
                        union.push([a, b]);
                    }
                }
                prior.span_intervals = Some(union);
            } else {
                l.span_intervals = Some(l.spans());
                merged.push(l);
            }
        }
        merged.sort_by(|a, b| {
            (b.gradient_ratio * b.fraction()).total_cmp(&(a.gradient_ratio * a.fraction()))
        });
        let start = lines[s].len();
        for mut l in merged {
            if lines[s].iter().any(|p| {
                (l.midpoint_pixel - p.midpoint_pixel).abs() < 2.
                    && (l.angle_degrees - p.angle_degrees).abs() <= 0.75
            }) {
                continue;
            }
            l.rank = evidence.lines[SIDES[s]].len() + lines[s].len() - start;
            lines[s].push(l);
            if lines[s].len() == start + 2 {
                break;
            }
        }
    }
    let mut angles: Vec<_> = lines
        .iter()
        .enumerate()
        .flat_map(|(s, ls)| ls.iter().map(move |l| l.physical_angle(s)))
        .collect();
    angles.sort_by(f64::total_cmp);
    angles.dedup();
    let mut anchors = angles.clone();
    for (i, a) in angles.iter().enumerate() {
        for b in &angles[i + 1..] {
            if (a - b).abs() <= 1.5 {
                anchors.push((a + b) / 2.);
            }
        }
    }
    anchors.sort_by(f64::total_cmp);
    anchors.dedup();
    let mut families = BTreeMap::new();
    for anchor in anchors {
        let options: [Vec<Option<usize>>; 4] = std::array::from_fn(|s| {
            std::iter::once(None)
                .chain(
                    lines[s]
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| (l.physical_angle(s) - anchor).abs() <= 0.75)
                        .map(|(i, _)| Some(i)),
                )
                .collect()
        });
        for &a in &options[0] {
            for &b in &options[1] {
                for &c in &options[2] {
                    for &d in &options[3] {
                        let indices = [a, b, c, d];
                        let count = indices.iter().flatten().count();
                        if count < 2 || families.contains_key(&indices) {
                            continue;
                        }
                        let chosen: Vec<_> = indices
                            .iter()
                            .enumerate()
                            .filter_map(|(s, i)| i.map(|i| (s, &lines[s][i])))
                            .collect();
                        let mean = chosen
                            .iter()
                            .map(|(s, l)| l.physical_angle(*s))
                            .sum::<f64>()
                            / count as f64;
                        let spread = chosen
                            .iter()
                            .map(|(s, l)| (l.physical_angle(*s) - mean).powi(2))
                            .sum::<f64>()
                            / count as f64;
                        let strength = chosen
                            .iter()
                            .map(|(_, l)| {
                                (l.gradient_ratio * if l.partial_span { l.fraction() } else { 1. })
                                    .ln()
                            })
                            .sum::<f64>()
                            / count as f64;
                        let mut score =
                            strength.exp() * count as f64 * (-spread / 0.75f64.powi(2)).exp();
                        let surfaces: Vec<_> = chosen
                            .iter()
                            .filter_map(|(_, l)| {
                                l.outside_surface.as_ref().map(|s| s.median_camera_rgb)
                            })
                            .collect();
                        if surfaces.len() >= 2 {
                            let colour: [f64; 3] = std::array::from_fn(|ch| {
                                surfaces.iter().map(|v| v[ch]).sum::<f64>() / surfaces.len() as f64
                            });
                            let scale = colour
                                .iter()
                                .map(|v| v * v)
                                .sum::<f64>()
                                .sqrt()
                                .max(1. / 65535.);
                            let delta = (surfaces
                                .iter()
                                .map(|v| {
                                    v.iter()
                                        .zip(colour)
                                        .map(|(a, b)| (a - b).powi(2))
                                        .sum::<f64>()
                                })
                                .sum::<f64>()
                                / surfaces.len() as f64)
                                .sqrt()
                                / scale;
                            score *= (-4. * delta).exp();
                        }
                        score *= surfaces.len() as f64 / count as f64;
                        let complete = count == 4 && chosen.iter().all(|(_, l)| !l.partial_span);
                        families.insert(
                            indices,
                            Family {
                                indices,
                                score,
                                complete,
                            },
                        );
                    }
                }
            }
        }
    }
    let keys: BTreeSet<_> = families.keys().copied().collect();
    let mut maximal: Vec<_> = families
        .into_iter()
        .filter(|(k, _)| {
            !keys
                .iter()
                .any(|other| k != other && k.iter().zip(other).all(|(a, b)| a.is_none() || a == b))
        })
        .map(|(_, v)| v)
        .collect();
    maximal.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.indices.cmp(&b.indices))
    });
    Ok(Group {
        lines,
        families: maximal,
    })
}
