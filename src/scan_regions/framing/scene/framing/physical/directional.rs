//! Bounded, noise-relative directional evidence; material identity is not a gradient sum.
use super::*;

#[cfg(test)]
pub(super) mod inspection;

#[derive(Clone, Copy, Serialize)]
struct Cell {
    value: f64,
    tangent: f64,
    curvature: f64,
    quantization: f64,
    role: Role,
    rgb: [u16; 3],
}

#[derive(Serialize)]
pub(super) struct Station {
    pub(super) index: usize,
    along: f64,
    contrast: f64,
    pub(super) noise: f64,
    pub(super) tangent: f64,
    best_offset: i32,
    reversed_offset: i32,
    z: f64,
    broad_base: bool,
    image_continuation: bool,
    exterior: Option<f64>,
    interior: Option<f64>,
    exterior_noise: f64,
    attenuated_exterior: bool,
    separated: bool,
    base_prefix: bool,
}

#[derive(Serialize)]
pub(super) struct Report {
    pub(super) measured: bool,
    pub(super) contradiction: bool,
    pub(super) rank: f64,
    old_measured: bool,
    pub(super) aggregate_z: f64,
    pub(super) blocks: Vec<f64>,
    pub(super) supporting_blocks: usize,
    pub(super) continuation_bins: [usize; 8],
    pub(super) structure_bins: [usize; 8],
    pub(super) reversed_bins: [usize; 8],
    pub(super) contradictory_stations: usize,
    pub(super) stations: Vec<Station>,
}

fn cell(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    p: [f64; 2],
    tangent: [f64; 2],
    inward: bool,
) -> Option<Cell> {
    if inward && !in_hypothesis(p, frame, Some(side)) {
        return None;
    }
    let (role, rgb) = role_at(capture, roles, p)?;
    if !matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible) {
        return None;
    }
    logs(rgb)?;
    let mut values = vec![];
    for distance in [-1., 0., 1.] {
        let q = std::array::from_fn(|axis| p[axis] + distance * tangent[axis]);
        if inward && !in_hypothesis(q, frame, Some(side)) {
            continue;
        }
        let (r, v) = role_at(capture, roles, q)?;
        if !matches!(r, Role::BaseCompatible | Role::AttenuatedCompatible) {
            return None;
        }
        values.push(logs(v)?.iter().sum::<f64>() / 3.);
    }
    if values.len() != 3 {
        return None;
    }
    Some(Cell {
        value: values.iter().sum::<f64>() / 3.,
        tangent: (values[2] - values[0]) * 0.5,
        curvature: (values[0] - 2. * values[1] + values[2]) / 6f64.sqrt(),
        quantization: rgb
            .iter()
            .map(|&value| 1. / (f64::from(value) * std::f64::consts::LN_2))
            .sum::<f64>()
            / 3.,
        role,
        rgb,
    })
}

fn value(profile: &[Option<Cell>; 12], distance: f64) -> Option<Cell> {
    let index = (distance + 5.5).round() as i32;
    (0..12)
        .contains(&index)
        .then(|| profile[index as usize])
        .flatten()
}

fn average(profile: &[Option<Cell>; 12], positions: &[f64]) -> Option<f64> {
    let values: Vec<_> = positions
        .iter()
        .filter_map(|&distance| value(profile, distance).map(|v| v.value))
        .collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn gradient(profile: &[Option<Cell>; 12], offset: f64) -> Option<f64> {
    Some(
        average(profile, &[offset + 0.5, offset + 1.5])?
            - average(profile, &[offset - 0.5, offset - 1.5])?,
    )
}

fn profile(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    p: [f64; 2],
    normal: [f64; 2],
) -> [Option<Cell>; 12] {
    let tangent = [-normal[1], normal[0]];
    let mut profile = [None; 12];
    for sign in [-1., 1.] {
        for step in 0..6 {
            let distance = sign * (step as f64 + 0.5);
            let q = std::array::from_fn(|axis| p[axis] - distance * normal[axis]);
            let Some(sample) = cell(capture, roles, frame, side, q, tangent, distance < 0.) else {
                break;
            };
            profile[(distance + 5.5).round() as usize] = Some(sample);
        }
    }
    profile
}

fn station(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    index: usize,
    exterior: Exterior,
    old: &Side,
) -> Option<Station> {
    let p = perimeter::point(frame, side, (index as f64 + 0.5) / 64.);
    role_at(capture, roles, p)?;
    let normal = perimeter::normal(frame.angle, side);
    let profile = profile(capture, roles, frame, side, p, normal);
    value(&profile, -0.5)?;
    value(&profile, 0.5)?;
    let cells: Vec<_> = profile.iter().flatten().collect();
    let quantization = median(cells.iter().map(|v| v.quantization));
    let noise = (1.4826 * median(cells.iter().map(|v| v.curvature.abs()))).max(quantization);
    let tangent = median(cells.iter().map(|v| v.tangent.abs()));
    let contrast = gradient(&profile, 0.)?;
    let controls: Vec<_> = [-3., 3.]
        .into_iter()
        .filter_map(|offset| gradient(&profile, offset))
        .collect();
    if controls.is_empty() {
        return None;
    }
    let background = median(controls.into_iter());
    let signal = if contrast > 0. {
        contrast.min((contrast - background).max(0.))
    } else {
        -(-contrast).min((background - contrast).max(0.))
    };
    let responses: Vec<_> = (-3..=3)
        .filter_map(|offset| gradient(&profile, f64::from(offset)).map(|g| (offset, g)))
        .collect();
    let best_offset: i32 = responses.iter().max_by(|a, b| a.1.total_cmp(&b.1))?.0;
    let reversed_offset: i32 = responses.iter().min_by(|a, b| a.1.total_cmp(&b.1))?.0;
    let mut z = signal / noise.hypot(tangent);
    if (z > 0. && best_offset.abs() > 1) || (z < 0. && reversed_offset.abs() > 1) {
        z = 0.;
    }
    let outside: Vec<_> = [0.5, 1.5, 2.5]
        .into_iter()
        .filter_map(|d| value(&profile, d))
        .collect();
    let far: Vec<_> = [3.5, 4.5, 5.5]
        .into_iter()
        .filter_map(|d| value(&profile, d))
        .collect();
    let broad_base = outside.iter().any(|v| v.role == Role::BaseCompatible);
    let base_prefix = outside.windows(2).any(|pair| {
        pair.iter()
            .all(|value| exterior.classify(value.rgb) == Material::Base)
    });
    let attenuated = outside
        .iter()
        .filter(|v| {
            v.role == Role::AttenuatedCompatible && exterior.classify(v.rgb) == Material::Photo
        })
        .count()
        >= 2;
    let separated = neighbors::holder_after_base(capture, roles, exterior, p, normal)
        || old.neighbor.as_ref().is_some_and(|n| {
            neighbors::gap_at(capture, roles, exterior, p, normal)
                .is_some_and(|gap| (gap - n.gap).abs() <= 3.)
        });
    // Point-raster station spacing alternates pixel steps. A smooth local gradient
    // therefore bounds phase-induced curvature; it is not photographic texture.
    let exterior_phase = if far.is_empty() {
        0.
    } else {
        median(far.iter().map(|v| v.tangent.abs()))
            .hypot(gradient(&profile, 4.).unwrap_or(0.) * 0.5)
    };
    let exterior_noise = (1.4826 * median(outside.iter().map(|v| v.curvature.abs())))
        .max(quantization)
        .hypot(exterior_phase);
    Some(Station {
        index,
        along: p[usize::from(side < 2)],
        contrast,
        noise,
        tangent,
        best_offset,
        reversed_offset,
        z: z.clamp(-4., 4.),
        broad_base,
        image_continuation: !separated
            && attenuated
            && z < 1.
            && contrast.abs() <= 2. * noise.hypot(tangent),
        exterior: (!far.is_empty()).then(|| median(far.iter().map(|v| v.value))),
        interior: average(&profile, &[-3.5, -4.5, -5.5]),
        exterior_noise,
        attenuated_exterior: far
            .iter()
            .any(|v| exterior.classify(v.rgb) == Material::Photo),
        separated,
        base_prefix,
    })
}

fn summarize(stations: Vec<Station>, old_measured: bool) -> (Report, bool) {
    let mut continuation_bins = [0; 8];
    let mut structure_bins = [0; 8];
    let mut reversed_bins = [0; 8];
    let mut contradictory = [false; 64];
    for sample in &stations {
        let reversed = sample.z < -3. && !sample.separated;
        continuation_bins[sample.index / 8] += usize::from(sample.image_continuation);
        reversed_bins[sample.index / 8] += usize::from(reversed);
        contradictory[sample.index] = sample.image_continuation || reversed;
    }
    for triplet in stations.windows(3) {
        let [a, b, c] = [&triplet[0], &triplet[1], &triplet[2]];
        // Texture after an observed base strip belongs to another exterior region;
        // it is not by itself evidence that the target picture continues across this edge.
        if c.index != a.index + 2 || b.separated || b.base_prefix || !b.attenuated_exterior {
            continue;
        }
        if let (Some(x), Some(y), Some(z)) = (a.exterior, b.exterior, c.exterior) {
            let noise = median([a.exterior_noise, b.exterior_noise, c.exterior_noise].into_iter());
            let outer = x - 2. * y + z;
            let correlated = match (a.interior, b.interior, c.interior) {
                (Some(u), Some(v), Some(w)) => {
                    let inner = u - 2. * v + w;
                    inner * outer > 0. && inner.abs() > 2. * noise
                }
                _ => false,
            };
            if outer.abs() > 6. * noise || (outer.abs() > 4. * noise && correlated) {
                structure_bins[b.index / 8] += 1;
                contradictory[b.index] = true;
            }
        }
    }
    let bad: [bool; 8] = std::array::from_fn(|i| {
        continuation_bins[i] >= 3 || structure_bins[i] >= 3 || reversed_bins[i] >= 3
    });
    // Distributed conflicts cannot disappear merely because supporting patches separate them.
    let contradictory_stations = contradictory.into_iter().filter(|&bad| bad).count();
    let contradiction =
        contradictory_stations >= 12 || bad.windows(2).any(|pair| pair[0] && pair[1]);
    let mut groups: [Vec<&Station>; 8] = std::array::from_fn(|_| vec![]);
    if let (Some(first), Some(last)) = (stations.first(), stations.last()) {
        let span = (last.index - first.index + 1) as f64;
        for sample in &stations {
            let index = (((sample.index - first.index) as f64 / span * 8.).floor() as usize).min(7);
            groups[index].push(sample);
        }
    }
    let blocks: Vec<_> = groups
        .iter()
        .filter(|group| group.len() >= 2)
        .map(|group| {
            let span = (group.last().unwrap().along - group[0].along).abs();
            let effective = (span / 3.).max(1.).min(group.len() as f64);
            (group.iter().map(|s| s.z).sum::<f64>() / group.len() as f64 * effective.sqrt())
                .clamp(-4., 4.)
        })
        .collect();
    let aggregate_z = if blocks.is_empty() {
        0.
    } else {
        blocks.iter().sum::<f64>() / (blocks.len() as f64).sqrt()
    };
    let supporting_blocks = blocks.iter().filter(|&&z| z > 1.).count();
    let base = stations.iter().filter(|s| s.broad_base).count();
    let positive = stations
        .iter()
        .filter(|s| s.z > 0. && s.best_offset.abs() <= 1)
        .count();
    let measured = stations.len() >= 12
        && positive >= 12
        && base >= 12
        && supporting_blocks >= 3
        && aggregate_z >= 3.;
    let rank = base as f64 / stations.len().max(1) as f64
        + (aggregate_z.max(0.) / 4.).atan() / std::f64::consts::PI;
    (
        Report {
            measured,
            contradiction,
            rank,
            old_measured,
            aggregate_z,
            blocks,
            supporting_blocks,
            continuation_bins,
            structure_bins,
            reversed_bins,
            contradictory_stations,
            stations,
        },
        contradiction,
    )
}

pub(super) fn measure(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    exterior: Option<Exterior>,
) -> Side {
    let mut result = perimeter::measure(capture, roles, frame, side, exterior);
    let old_measured = result.measured();
    let stations = exterior.map_or_else(Vec::new, |exterior| {
        (0..64)
            .filter_map(|index| station(capture, roles, frame, side, index, exterior, &result))
            .collect()
    });
    let (report, contradiction) = summarize(stations, old_measured);
    result.contradiction = result.support.contradicted() || contradiction;
    result.directional = Some(report);
    result
}
