use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) enum Kind {
    Picture,
    FilmLimit,
    HolderLimit,
    Unresolved,
}

#[derive(Clone)]
pub(super) struct Pair {
    pub(super) along: f64,
    pub(super) density: [f64; 3],
    pub(super) base_outside: bool,
}

#[derive(Default, Clone, Serialize)]
pub(super) struct Support {
    pub(super) stations: usize,
    pub(super) paired: usize,
    pub(super) positive: usize,
    pub(super) base_outside: usize,
    pub(super) inward_nonfilm: usize,
    pub(super) film_limit: usize,
    pub(super) holder_limit: usize,
    pub(super) outside_unavailable: usize,
    #[serde(skip)]
    pub(super) pairs: Vec<Pair>,
}

impl Support {
    pub(super) fn kind(&self) -> Kind {
        let covers =
            |count: usize| self.stations >= 12 && count as f64 >= 0.75 * self.stations as f64;
        if covers(self.holder_limit) {
            Kind::HolderLimit
        } else if covers(self.film_limit) {
            Kind::FilmLimit
        } else if self.paired >= 12 && self.positive as f64 >= 0.75 * self.paired as f64 {
            Kind::Picture
        } else {
            Kind::Unresolved
        }
    }
    pub(super) fn contradicted(&self) -> bool {
        self.stations >= 12 && self.inward_nonfilm as f64 >= 0.75 * self.stations as f64
    }
}

pub(super) fn role_at(capture: &Capture, roles: &[Role], p: [f64; 2]) -> Option<(Role, [u16; 3])> {
    let p = p.map(|v| v.floor() as i32);
    let rgb = capture.rgb(p)?;
    Some((
        roles[p[1] as usize * capture.size[0] as usize + p[0] as usize],
        rgb,
    ))
}

pub(super) fn in_hypothesis(p: [f64; 2], frame: &PictureGeometry, skip: Option<usize>) -> bool {
    let uv = project(p, frame.angle);
    frame.sides.iter().enumerate().all(|(side, bound)| {
        skip == Some(side)
            || bound.is_none_or(|bound| {
                if side == 0 || side == 2 {
                    uv[usize::from(side >= 2)] >= bound - 1e-8
                } else {
                    uv[usize::from(side >= 2)] <= bound + 1e-8
                }
            })
    })
}

pub(super) fn measure(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    line: &Line,
) -> Support {
    let other = usize::from(side < 2);
    let positions = super::super::super::super::indices(capture.size[other], 32)
        .into_iter()
        .filter(|&index| line.observed(f64::from(index) / f64::from(capture.size[other] - 1)))
        .map(|index| f64::from(index) + 0.5);
    measure_positions(capture, roles, frame, side, positions)
}

fn measure_positions(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    positions: impl Iterator<Item = f64>,
) -> Support {
    let mut result = Support::default();
    let Some(bound) = frame.sides[side] else {
        return result;
    };
    let other = usize::from(side < 2);
    let (s, c) = frame.angle.to_radians().sin_cos();
    let normal = if side < 2 { [c, s] } else { [-s, c] };
    let direction = if side == 0 || side == 2 { 1. } else { -1. };
    for along in positions {
        // The common pose supplies direction; this function measures its own visible support.
        let point = if side < 2 {
            [(bound - s * along) / c, along]
        } else {
            [along, (bound + s * along) / c]
        };
        if !in_hypothesis(point, frame, Some(side)) {
            continue;
        }
        if role_at(capture, roles, point).is_none() {
            continue;
        }
        let sample = |sign: f64| {
            let mut values = vec![];
            let mut first = None;
            for distance in [0.5, 1.5, 2.5] {
                let p = std::array::from_fn(|axis| {
                    point[axis] + sign * direction * distance * normal[axis]
                });
                let Some((role, rgb)) = role_at(capture, roles, p) else {
                    break;
                };
                if first.is_none() {
                    first = Some(role);
                }
                if !matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible)
                    || (sign > 0. && !in_hypothesis(p, frame, Some(side)))
                {
                    break;
                }
                values.push(rgb);
            }
            let mean = (!values.is_empty()).then(|| {
                std::array::from_fn::<_, 3, _>(|ch| {
                    values.iter().map(|v| f64::from(v[ch])).sum::<f64>() / values.len() as f64
                })
            });
            (first, mean)
        };
        let (inside, inside_rgb) = sample(1.);
        let (outside, outside_rgb) = sample(-1.);
        if inside == outside && matches!(inside, Some(Role::Holder | Role::BrighterThanBase)) {
            continue;
        }
        result.stations += 1;
        result.inward_nonfilm += usize::from(matches!(
            inside,
            Some(Role::Holder | Role::BrighterThanBase)
        ));
        result.holder_limit += usize::from(outside == Some(Role::Holder) && inside_rgb.is_some());
        result.film_limit +=
            usize::from(outside == Some(Role::BrighterThanBase) && inside_rgb.is_some());
        let (Some(a), Some(b)) = (inside_rgb, outside_rgb) else {
            result.outside_unavailable += 1;
            continue;
        };
        result.paired += 1;
        let positive = (0..3).all(|ch| a[ch] <= b[ch]) && (0..3).any(|ch| a[ch] < b[ch]);
        result.positive += usize::from(positive);
        result.base_outside += usize::from(positive && outside == Some(Role::BaseCompatible));
        result.pairs.push(Pair {
            along: along / f64::from(capture.size[other]),
            density: std::array::from_fn(|channel| (b[channel] / a[channel]).log2()),
            base_outside: outside == Some(Role::BaseCompatible),
        });
    }
    result
}

pub(super) fn holder_limits(scene: &Scene, angle: f64) -> [Option<f64>; 4] {
    let [w, h] = scene.size.map(|v| v as usize);
    std::array::from_fn(|side| {
        let (along, cross) = if side < 2 { (h, w) } else { (w, h) };
        let points = (along / 4..3 * along / 4).filter_map(|index| {
            let depth = (0..cross)
                .take_while(|&d| {
                    let (x, y) = match side {
                        0 => (d, index),
                        1 => (w - 1 - d, index),
                        2 => (index, d),
                        _ => (index, h - 1 - d),
                    };
                    scene.holder[y * w + x]
                })
                .count();
            if depth == 0 || depth >= cross / 3 {
                return None;
            }
            let across = if side == 0 || side == 2 {
                depth as f64
            } else {
                (cross - depth) as f64
            };
            Some(if side < 2 {
                [across, index as f64 + 0.5]
            } else {
                [index as f64 + 0.5, across]
            })
        });
        crate::scan_regions::opening::rail(scene.size, angle, side, points)
    })
}

pub(super) fn restricted(frame: &PictureGeometry, limits: [Option<f64>; 4]) -> PictureGeometry {
    let mut result = frame.clone();
    for (side, &limit) in limits.iter().enumerate() {
        if let Some(limit) = limit {
            result.sides[side] = Some(frame.sides[side].map_or(limit, |value| {
                if side == 0 || side == 2 {
                    value.max(limit)
                } else {
                    value.min(limit)
                }
            }));
        }
    }
    result
}
