use super::*;

pub(super) fn normal(angle: f64, side: usize) -> [f64; 2] {
    let (s, c) = angle.to_radians().sin_cos();
    let sign = if side == 0 || side == 2 { 1. } else { -1. };
    if side < 2 {
        [sign * c, sign * s]
    } else {
        [-sign * s, sign * c]
    }
}

pub(super) fn point(frame: &PictureGeometry, side: usize, fraction: f64) -> [f64; 2] {
    let bounds = frame.sides.map(Option::unwrap);
    let other = usize::from(side < 2);
    let along = bounds[2 * other] + fraction * (bounds[2 * other + 1] - bounds[2 * other]);
    project(
        if side < 2 {
            [bounds[side], along]
        } else {
            [along, bounds[side]]
        },
        -frame.angle,
    )
}

#[derive(Serialize)]
pub(super) struct Side {
    pub(super) available: usize,
    pub(super) base: usize,
    pub(super) photo: usize,
    pub(super) unknown: usize,
    pub(super) reverse: usize,
    pub(super) base_to_holder: usize,
    pub(super) bad_bins: [bool; 8],
    pub(super) contradiction: bool,
    pub(super) exterior_supported: bool,
    pub(super) support: Support,
    pub(super) pooled: Option<super::super::pooled::Pooled>,
    pub(super) neighbor: Option<Neighbor>,
    pub(super) neighbor_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) directional: Option<super::directional::Report>,
}
impl Side {
    pub(super) fn measured(&self) -> bool {
        !self.contradiction
            && self
                .directional
                .as_ref()
                .map_or(self.exterior_supported && self.pooled.is_some(), |report| {
                    report.measured
                })
    }
    pub(super) fn score(&self) -> f64 {
        if let Some(report) = &self.directional {
            return report.rank;
        }
        let material = self.base as f64 / (self.base + self.photo).max(1) as f64;
        material
            + self
                .pooled
                .as_ref()
                .map_or(0., |value| value.rank_signal.atan() / std::f64::consts::PI)
    }
}

pub(super) fn measure(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    exterior: Option<Exterior>,
) -> Side {
    let mut result = Side {
        available: 0,
        base: 0,
        photo: 0,
        unknown: 0,
        reverse: 0,
        base_to_holder: 0,
        bad_bins: [false; 8],
        contradiction: false,
        exterior_supported: false,
        support: Support::default(),
        pooled: None,
        neighbor: None,
        neighbor_required: false,
        directional: None,
    };
    let Some(exterior) = exterior else {
        return result;
    };
    let mut base_bins = [0usize; 8];
    let mut photo_bins = [0usize; 8];
    let mut reverse_bins = [0usize; 8];
    let mut pending = vec![];
    let mut gaps = vec![];
    let inward = normal(frame.angle, side);
    for station in 0..64 {
        let p = point(frame, side, (station as f64 + 0.5) / 64.);
        if role_at(capture, roles, p).is_none() {
            continue;
        }
        let sample = |sign: f64| {
            let mut samples = vec![];
            for distance in [0.5, 1.5, 2.5] {
                let q = std::array::from_fn(|axis| p[axis] + sign * distance * inward[axis]);
                let Some((role, rgb)) = role_at(capture, roles, q) else {
                    break;
                };
                if !matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible) {
                    if samples.is_empty() {
                        samples.push((role, rgb));
                    }
                    break;
                }
                if sign > 0. && !in_hypothesis(q, frame, Some(side)) {
                    break;
                }
                samples.push((role, rgb));
            }
            samples
        };
        let inside = sample(1.);
        let outside = sample(-1.);
        if inside.is_empty() {
            continue;
        }
        result.support.stations += 1;
        if !matches!(
            inside[0].0,
            Role::BaseCompatible | Role::AttenuatedCompatible
        ) {
            result.support.inward_nonfilm += 1;
            continue;
        }
        if outside.is_empty() {
            result.unknown += 1;
            continue;
        }
        if !matches!(
            outside[0].0,
            Role::BaseCompatible | Role::AttenuatedCompatible
        ) {
            result.unknown += 1;
            result.support.film_limit += usize::from(outside[0].0 == Role::BrighterThanBase);
            result.support.holder_limit += usize::from(outside[0].0 == Role::Holder);
            continue;
        }
        result.available += 1;
        let classes: Vec<_> = outside
            .iter()
            .map(|(_, rgb)| exterior.classify(*rgb))
            .collect();
        // One near sample can straddle a fuzzy boundary. Do not skip a continuing picture
        // or jump a missing/holder/light interval to find convenient base farther away.
        let start = if classes.first() == Some(&Material::Photo)
            && classes.get(1) == Some(&Material::Base)
        {
            1
        } else {
            0
        };
        let length = classes[start..]
            .iter()
            .take_while(|&&value| value == Material::Base)
            .count();
        if length == 0 {
            if classes[0] == Material::Photo {
                result.photo += 1;
                photo_bins[station / 8] += 1;
            } else {
                result.unknown += 1;
            }
            continue;
        }
        let meets_photo = classes[start + length..].contains(&Material::Photo);
        let holder_transition =
            meets_photo && super::neighbors::holder_after_base(capture, roles, exterior, p, inward);
        let gap = super::neighbors::gap_at(capture, roles, exterior, p, inward);
        if !holder_transition && let Some(gap) = gap {
            gaps.push((station, gap));
        }
        result.base_to_holder += usize::from(holder_transition);
        if meets_photo && !holder_transition {
            pending.push(station);
        } else {
            result.base += 1;
            base_bins[station / 8] += 1;
        }
        let a: [f64; 3] = std::array::from_fn(|ch| {
            inside
                .iter()
                .map(|(_, rgb)| f64::from(rgb[ch]))
                .sum::<f64>()
                / inside.len() as f64
        });
        let b: [f64; 3] = std::array::from_fn(|ch| {
            outside[start..start + length]
                .iter()
                .map(|(_, rgb)| f64::from(rgb[ch]))
                .sum::<f64>()
                / length as f64
        });
        let density = std::array::from_fn(|ch| (b[ch] / a[ch]).log2());
        let reversed = density.iter().sum::<f64>() / 3. < -3. * median(exterior.spread.into_iter());
        result.reverse += usize::from(reversed);
        reverse_bins[station / 8] += usize::from(reversed);
        result.support.paired += 1;
        result.support.positive +=
            usize::from(density.iter().all(|&v| v >= 0.) && density.iter().any(|&v| v > 0.));
        result.support.base_outside += 1;
        // Normalized source along-coordinate keeps the existing coherent signal pooling reusable.
        result.support.pairs.push(Pair {
            along: p[usize::from(side < 2)] / f64::from(capture.size[usize::from(side < 2)]),
            density,
            base_outside: true,
        });
    }
    result.neighbor = super::neighbors::fit(frame, side, &gaps);
    for station in pending {
        if let Some(neighbor) = &result.neighbor
            && gaps
                .iter()
                .any(|&(s, gap)| s == station && (gap - neighbor.gap).abs() <= 3.)
        {
            result.base += 1;
            base_bins[station / 8] += 1;
            result.neighbor_required = true;
        } else {
            result.photo += 1;
            photo_bins[station / 8] += 1;
        }
    }
    for bin in 0..8 {
        result.bad_bins[bin] = photo_bins[bin] >= 3 || reverse_bins[bin] >= 3;
    }
    let coherent = result.bad_bins.windows(2).any(|bins| bins[0] && bins[1]);
    result.contradiction = result.support.contradicted()
        || coherent
        || (result.photo >= 12 && result.photo * 4 > result.base + result.photo);
    result.exterior_supported = result.base >= 12
        && result.base * 3 >= (result.base + result.photo) * 2
        && base_bins.iter().filter(|&&count| count >= 2).count() >= 2;
    result.pooled = super::super::pooled::pooled(&result.support);
    result
}
