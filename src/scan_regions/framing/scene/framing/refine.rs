//! Small integer-fine-pixel corrections on already associated edge portions, never a new detector.
use super::{light::ExteriorLight, *};
use std::time::Instant;

const STATIONS: usize = 16;
const RADIUS: i32 = 12;
const BAND: i32 = 4;
const SAMPLES: usize = (2 * (RADIUS + BAND)) as usize;

#[derive(Default)]
struct Constraints {
    holder_limits: [Option<f64>; 4],
    separators: Vec<Separator>,
}

fn valid(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    limits: [Option<f64>; 4],
    separators: &[Separator],
    exterior_light: &ExteriorLight,
) -> Option<Crop> {
    let center = [
        f64::from(capture.size[0]) / 2.,
        f64::from(capture.size[1]) / 2.,
    ];
    if !in_hypothesis(center, frame, None)
        || crosses_separator(frame, limits, separators)
        || exterior_light.intersects(frame)
    {
        return None;
    }
    let crop = visible_crop(capture.area, &restricted(frame, limits))?;
    (nonfilm_fraction(capture, roles, crop) <= 0.02).then_some(crop)
}

#[derive(Clone, Copy)]
pub(crate) struct Fine<'a> {
    pub(crate) size: [u32; 2],
    pub(crate) rgb: &'a [[u16; 3]],
}

#[derive(Clone, Serialize)]
pub(crate) struct Edge {
    pub(crate) side: usize,
    pub(crate) status: &'static str,
    pub(crate) offset_fine: Option<i32>,
    pub(crate) range_fine: Option<[i32; 2]>,
    pub(crate) width_offsets: Vec<i32>,
    pub(crate) stations: usize,
    pub(crate) along: Vec<f64>,
    pub(crate) rgb_queries: usize,
}

#[derive(Serialize)]
pub(crate) struct Report {
    pub(crate) applied: bool,
    pub(crate) status: &'static str,
    pub(crate) physical: PictureGeometry,
    pub(crate) crop: Crop,
    pub(crate) edges: Vec<Edge>,
    pub(crate) inferred_envelope_raster: Option<[f64; 2]>,
    pub(crate) uncertainty_unresolved: bool,
    pub(crate) profile_us: u128,
    pub(crate) validation_us: u128,
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    (values[(values.len() - 1) / 2] + values[values.len() / 2]) * 0.5
}

fn locations(choice: &Choice, side: usize) -> Vec<f64> {
    if let Some(span) = choice.pooled[side].as_ref().and_then(|v| v.rank_span) {
        return (0..STATIONS)
            .map(|i| span[0] + (span[1] - span[0]) * (i as f64 + 0.5) / STATIONS as f64)
            .collect();
    }
    let pairs = &choice.support[side].pairs;
    if pairs.is_empty() {
        return vec![];
    }
    (0..pairs.len().min(STATIONS))
        .map(|i| pairs[i * pairs.len() / pairs.len().min(STATIONS)].along)
        .collect()
}

fn direction(frame: &PictureGeometry, side: usize, fine_size: [u32; 2]) -> ([f64; 2], f64) {
    let (s, c) = frame.angle.to_radians().sin_cos();
    let normal = if side < 2 { [c, s] } else { [-s, c] };
    let scale: [f64; 2] =
        std::array::from_fn(|axis| f64::from(fine_size[axis]) / f64::from(frame.size[axis]));
    (normal, (normal[0] * scale[0]).hypot(normal[1] * scale[1]))
}

fn locate(profiles: &[[f64; SAMPLES]], width: i32) -> Option<i32> {
    let contrast = |row: &[f64; SAMPLES], offset: i32| {
        let index = (offset + RADIUS + BAND) as usize;
        (row[index - width as usize..index].iter().sum::<f64>()
            - row[index..index + width as usize].iter().sum::<f64>())
            / f64::from(width)
    };
    let shoulders: Vec<_> = profiles
        .iter()
        .map(|row| {
            median(
                [-RADIUS, -RADIUS + 1, RADIUS - 1, RADIUS]
                    .into_iter()
                    .map(|j| contrast(row, j)),
            )
        })
        .collect();
    let curve: Vec<_> = (-RADIUS..=RADIUS)
        .map(|offset| {
            median(
                profiles
                    .iter()
                    .zip(&shoulders)
                    .map(|(row, base)| contrast(row, offset) - base),
            )
        })
        .collect();
    let peak = (0..curve.len()).max_by(|&a, &b| curve[a].total_cmp(&curve[b]))?;
    // A maximum at the search cap is not a local refinement; a flat/ramp has no isolated step.
    if peak == 0 || peak + 1 == curve.len() || curve[peak] <= 1e-8 {
        return None;
    }
    let offset = peak as i32 - RADIUS;
    let spread = 1.4826
        * median(
            profiles
                .iter()
                .zip(&shoulders)
                .map(|(row, base)| (contrast(row, offset) - base - curve[peak]).abs()),
        );
    // Exploratory noise test, not calibrated physical confidence after candidate selection.
    if curve[peak] <= 4. * 1.2533 * spread / (profiles.len() as f64).sqrt()
        || median(profiles.iter().map(|row| contrast(row, offset))) <= 0.
    {
        return None;
    }
    Some(offset)
}

fn edge(capture: &Capture, roles: &[Role], fine: Fine<'_>, choice: &Choice, side: usize) -> Edge {
    let frame = &choice.physical;
    let fine_size = fine.size;
    let (normal, normal_scale) = direction(frame, side, fine_size);
    let tangent = [-normal[1], normal[0]];
    let scale: [f64; 2] =
        std::array::from_fn(|axis| f64::from(fine_size[axis]) / f64::from(frame.size[axis]));
    let tangent_scale = (tangent[0] * scale[0]).hypot(tangent[1] * scale[1]);
    let sign = if side == 0 || side == 2 { 1. } else { -1. };
    let (s, c) = frame.angle.to_radians().sin_cos();
    let mut result = Edge {
        side,
        status: "unavailable",
        offset_fine: None,
        range_fine: None,
        width_offsets: vec![],
        stations: 0,
        along: vec![],
        rgb_queries: 0,
    };
    let mut profiles = vec![];
    for along in locations(choice, side) {
        let along = along * f64::from(frame.size[usize::from(side < 2)]);
        let bound = frame.sides[side].unwrap();
        let point = if side < 2 {
            [(bound - s * along) / c, along]
        } else {
            [along, (bound + s * along) / c]
        };
        if !in_hypothesis(point, frame, Some(side)) {
            continue;
        }
        let sample = |offset: f64, along_offset: f64, count: &mut usize| -> Option<f64> {
            let p: [f64; 2] = std::array::from_fn(|axis| {
                point[axis]
                    + sign * normal[axis] * offset / normal_scale
                    + tangent[axis] * along_offset / tangent_scale
            });
            if !in_hypothesis(p, frame, Some(side))
                || !matches!(
                    role_at(capture, roles, p)?.0,
                    Role::BaseCompatible | Role::AttenuatedCompatible
                )
            {
                return None;
            }
            let pixel =
                std::array::from_fn::<_, 2, _>(|axis| (p[axis] * scale[axis]).floor() as i32);
            if pixel[0] < 0
                || pixel[1] < 0
                || pixel[0] >= fine_size[0] as i32
                || pixel[1] >= fine_size[1] as i32
            {
                return None;
            }
            *count += 1;
            let rgb = fine.rgb[pixel[1] as usize * fine_size[0] as usize + pixel[0] as usize];
            if rgb.iter().any(|&value| value <= 1 || value == u16::MAX) {
                return None;
            }
            Some(
                rgb.into_iter()
                    .map(|value| f64::from(value).log2())
                    .sum::<f64>()
                    / 3.,
            )
        };
        let mut profile = [0.; SAMPLES];
        let mut available = true;
        for (index, value) in profile.iter_mut().enumerate() {
            let offset = index as f64 - f64::from(RADIUS + BAND) + 0.5;
            let sum = [-1., 0., 1.].into_iter().try_fold(0., |sum, along| {
                sample(offset, along, &mut result.rgb_queries).map(|value| sum + value)
            });
            let Some(sum) = sum else {
                available = false;
                break;
            };
            *value = sum / 3.;
        }
        if available {
            profiles.push(profile);
            result
                .along
                .push(along / f64::from(frame.size[usize::from(side < 2)]));
        }
    }
    result.stations = profiles.len();
    if profiles.len() < 8 {
        return result;
    }
    for width in [2, 4] {
        let Some(offset) = locate(&profiles, width) else {
            result.status = "ambiguous_profile";
            return result;
        };
        result.width_offsets.push(offset);
    }
    let low = *result.width_offsets.iter().min().unwrap();
    let high = *result.width_offsets.iter().max().unwrap();
    if high - low > 2 {
        result.status = "scale_disagreement";
        return result;
    }
    result.status = "stable_local_step";
    result.offset_fine = Some(((low + high) as f64 / 2.).round() as i32);
    // One fine sample beyond the two scale estimates expresses fuzz/quantization, not a CI.
    result.range_fine = Some([low - 1, high + 1]);
    result
}

pub(crate) fn needed(choice: &Choice, baseline: &Areas) -> bool {
    (0..4).any(|side| {
        choice.measured[side] && (!baseline.picture_observed[side] || baseline.picture_weak[side])
    })
}

fn run(
    capture: &Capture,
    roles: &[Role],
    fine: Fine<'_>,
    choice: &Choice,
    reference: &Reference,
    previous: &Constraints,
    baseline: &Areas,
) -> Result<Report, String> {
    let fine_size = fine.size;
    if fine.rgb.len() != fine_size[0] as usize * fine_size[1] as usize
        || roles.len() != capture.rgb.len()
        || choice.physical.size != capture.size
        || fine_size
            .iter()
            .zip(capture.size)
            .any(|(&fine, coarse)| fine < coarse || fine > 4 * coarse)
    {
        return Err("fine refinement requires a compatible <=4x cached raster".into());
    }
    let started = Instant::now();
    let mut result = Report {
        applied: false,
        status: "no_stable_local_update",
        physical: choice.physical.clone(),
        crop: choice.crop,
        edges: vec![],
        inferred_envelope_raster: None,
        uncertainty_unresolved: choice.inferred.iter().any(|&value| value),
        profile_us: 0,
        validation_us: 0,
    };
    let mut updated = choice.physical.clone();
    let mut ranges = [[0.; 2]; 4];
    for (side, range) in ranges.iter_mut().enumerate() {
        let Some(bound) = updated.sides[side] else {
            continue;
        };
        *range = [bound; 2];
        if !choice.measured[side] {
            continue;
        }
        if baseline.picture_observed[side] && !baseline.picture_weak[side] {
            result.edges.push(Edge {
                side,
                status: "retained_strong_anchor",
                offset_fine: None,
                range_fine: None,
                width_offsets: vec![],
                stations: 0,
                along: vec![],
                rgb_queries: 0,
            });
            continue;
        }
        let estimate = edge(capture, roles, fine, choice, side);
        if let Some(offset) = estimate.offset_fine {
            let (_, scale) = direction(&updated, side, fine_size);
            let sign = if side == 0 || side == 2 { 1. } else { -1. };
            updated.sides[side] = Some(bound + sign * f64::from(offset) / scale);
            let proposed_range = estimate
                .range_fine
                .unwrap()
                .map(|offset| bound + sign * f64::from(offset) / scale);
            *range = [
                proposed_range[0].min(proposed_range[1]),
                proposed_range[0].max(proposed_range[1]),
            ];
        }
        result.edges.push(estimate);
    }
    result.profile_us = started.elapsed().as_micros();
    if !result
        .edges
        .iter()
        .any(|edge| edge.offset_fine.is_some_and(|offset| offset != 0))
    {
        return Ok(result);
    }
    let validation = Instant::now();
    if let Some(missing) = choice.inferred.iter().position(|&value| value) {
        let Some(shape) = reference.shape.as_ref() else {
            result.status = "retained_without_independent_ratio";
            return Ok(result);
        };
        updated.sides[missing] = None;
        let Ok(draft) = super::super::super::shape::complete(&updated, capture.area, shape) else {
            result.status = "retained_after_completion_check";
            return Ok(result);
        };
        updated = draft.geometry;
        let mut envelope = [f64::INFINITY, f64::NEG_INFINITY];
        for mask in 0..16 {
            let mut extreme = updated.clone();
            for side in 0..4 {
                extreme.sides[side] = if side == missing {
                    None
                } else {
                    Some(ranges[side][(mask >> side) & 1])
                };
            }
            if let Ok(draft) = super::super::super::shape::complete(&extreme, capture.area, shape) {
                envelope[0] = envelope[0].min(draft.inferred_band_raster[0]);
                envelope[1] = envelope[1].max(draft.inferred_band_raster[1]);
            }
        }
        if result.edges.iter().all(|edge| edge.offset_fine.is_some())
            && envelope.iter().all(|value| value.is_finite())
        {
            result.inferred_envelope_raster = Some(envelope);
            result.uncertainty_unresolved = false;
        }
    }
    let (s, c) = updated.angle.to_radians().sin_cos();
    let supported = result
        .edges
        .iter()
        .filter(|edge| edge.offset_fine.is_some())
        .all(|edge| {
            edge.along.iter().all(|along| {
                let along = along * f64::from(updated.size[usize::from(edge.side < 2)]);
                let bound = updated.sides[edge.side].unwrap();
                let point = if edge.side < 2 {
                    [(bound - s * along) / c, along]
                } else {
                    [along, (bound + s * along) / c]
                };
                in_hypothesis(point, &updated, Some(edge.side))
            })
        });
    if !supported {
        result.status = "retained_after_companion_check";
        result.inferred_envelope_raster = None;
        result.uncertainty_unresolved = true;
        result.validation_us = validation.elapsed().as_micros();
        return Ok(result);
    }
    let light = ExteriorLight::new(capture, roles);
    if let Some(crop) = valid(
        capture,
        roles,
        &updated,
        previous.holder_limits,
        &previous.separators,
        &light,
    ) {
        result.physical = updated;
        result.crop = crop;
        result.applied = true;
        result.status = "refined_selected_anchors";
    } else {
        result.status = "retained_after_geometry_check";
        result.inferred_envelope_raster = None;
        result.uncertainty_unresolved = choice.inferred.iter().any(|&value| value);
    }
    result.validation_us = validation.elapsed().as_micros();
    Ok(result)
}

#[cfg(test)]
mod limits;
#[cfg(test)]
mod tests;
