//! Side-search support and bounded joint hypothesis selection.
use super::*;

#[cfg(test)]
pub(super) const LIMIT: usize = 81;

pub(super) fn search_band(template: &Template, angle: f64, side: usize) -> Band {
    let axis = usize::from(side >= 2);
    let (sine, cosine) = (angle - template.pose.center).to_radians().sin_cos();
    let center_radius = cosine.abs() * template.centers[axis].radius
        + sine.abs() * template.centers[1 - axis].radius;
    let extent = if axis == 0 {
        template.width
    } else {
        template.height
    };
    let scale = model::axis_scales(angle, template.scale)[axis];
    // Project the same centre/physical-size box used by complete-frame validation.
    // A tighter marginal side fit must not censor otherwise admissible combinations.
    Band {
        center: template.frame(angle).sides[side].unwrap(),
        radius: center_radius + extent.radius / (2. * scale),
    }
}

#[derive(Serialize)]
#[cfg(test)]
pub(super) struct Search {
    locations: [usize; 4],
    combinations: usize,
    compatible: [usize; 2],
    shortlisted: [usize; 2],
}

#[cfg(test)]
pub(super) struct Candidate {
    pub(super) frame: PictureGeometry,
    pub(super) proposed: [bool; 4],
    band: Option<[f64; 2]>,
    pub(super) rank: f64,
}

#[cfg(test)]
fn retain_candidate(candidates: &mut Vec<Candidate>, candidate: Candidate) {
    let index = candidates.partition_point(|other| other.rank >= candidate.rank);
    if index < LIMIT {
        candidates.insert(index, candidate);
        candidates.truncate(LIMIT);
    }
}

#[cfg(test)]
pub(super) fn hypotheses(
    locations: &[Vec<(f64, f64)>; 4],
    template: &Template,
    reference: &Reference,
    area: SourceRegion,
    pose: f64,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<([Vec<Candidate>; 2], Search), String> {
    // Cheap joint feasibility precedes the expensive perimeter shortlist. Keeping only
    // locally best sides here can discard every physically compatible combination.
    assert!(locations.iter().all(|side| side.len() <= 41));
    let options: [Vec<_>; 4] = std::array::from_fn(|side| {
        std::iter::once(None)
            .chain(locations[side].iter().copied().map(Some))
            .collect()
    });
    let mut result: [Vec<Candidate>; 2] = std::array::from_fn(|_| vec![]);
    let mut search = Search {
        locations: locations.each_ref().map(Vec::len),
        combinations: 0,
        compatible: [0; 2],
        shortlisted: [0; 2],
    };
    let scales = model::axis_scales(pose, template.scale);
    for l in &options[0] {
        for r in &options[1] {
            if let (Some((left, _)), Some((right, _))) = (l, r)
                && !template.width.contains((right - left) * scales[0])
            {
                continue;
            }
            for t in &options[2] {
                for b in &options[3] {
                    search.combinations += 1;
                    if search.combinations.is_multiple_of(4096) {
                        checkpoint()?;
                    }
                    if let (Some((top, _)), Some((bottom, _))) = (t, b)
                        && !template.height.contains((bottom - top) * scales[1])
                    {
                        continue;
                    }
                    let values = [*l, *r, *t, *b];
                    let proposed = values.map(|v| v.is_some());
                    let count = proposed.iter().filter(|&&v| v).count();
                    if count < 3 {
                        continue;
                    }
                    let mut frame = PictureGeometry {
                        size: template.size,
                        angle: pose,
                        sides: values.map(|v| v.map(|(bound, _)| bound)),
                    };
                    let band = if count == 3 {
                        let Some(shape) = &reference.shape else {
                            continue;
                        };
                        let Ok(draft) =
                            super::super::super::super::shape::complete(&frame, area, shape)
                        else {
                            continue;
                        };
                        frame = draft.geometry;
                        Some(draft.inferred_band_raster)
                    } else {
                        None
                    };
                    if !template.compatible(&frame) {
                        continue;
                    }
                    let phase = usize::from(count == 3);
                    search.compatible[phase] += 1;
                    let rank = values.iter().flatten().map(|(_, score)| score).sum::<f64>()
                        - template.displacement(&frame);
                    retain_candidate(
                        &mut result[phase],
                        Candidate {
                            frame,
                            proposed,
                            band,
                            rank,
                        },
                    );
                }
            }
        }
    }
    search.shortlisted = result.each_ref().map(Vec::len);
    Ok((result, search))
}

#[cfg(test)]
pub(super) struct Context<'a> {
    pub(super) scene: &'a Scene,
    pub(super) roles: &'a [Role],
    pub(super) reference: &'a Reference,
    pub(super) template: &'a Template,
    pub(super) pose: f64,
    pub(super) mode: Mode,
}

#[cfg(test)]
pub(super) fn evaluate(
    mut report: Report,
    target: Target<'_>,
    context: Context<'_>,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Report, String> {
    let Context {
        scene,
        roles,
        reference,
        template,
        pose,
        mode,
    } = context;
    let locations = ranked_locations(target.capture, roles, &target, template, pose, mode);
    let (phases, search) = hypotheses(
        &locations,
        template,
        reference,
        target.capture.area,
        pose,
        &checkpoint,
    )?;
    report.joint_search = Some(search);
    let light = light::ExteriorLight::new(target.capture, roles);
    for candidates in phases {
        let mut best: Option<(f64, Choice, [Side; 4])> = None;
        let mut alternatives = vec![];
        for candidate in candidates {
            checkpoint()?;
            report.tested += 1;
            let (crop, sides) = match eligible_with(
                scene,
                target.capture,
                roles,
                &candidate.frame,
                Some(template),
                &light,
                mode,
            ) {
                Ok(value) => value,
                Err(reason) => {
                    reject(&mut report, reason);
                    continue;
                }
            };
            if (0..4).any(|side| candidate.proposed[side] && !sides[side].measured()) {
                reject(&mut report, "unidentified_anchor");
                continue;
            }
            let count = candidate.proposed.iter().filter(|&&v| v).count();
            let score = sides.iter().map(Side::score).sum::<f64>()
                - template.displacement(&candidate.frame);
            let choice = make_choice(
                candidate.frame,
                crop,
                &sides,
                candidate.proposed,
                candidate.band,
                mode.method(count, false),
                target.capture,
            );
            alternatives.push((count, score, choice.physical.sides));
            if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
                best = Some((score, choice, sides));
            }
        }
        if let Some((score, choice, sides)) = best {
            let count = choice.measured.iter().filter(|&&v| v).count();
            report.ambiguity =
                distinct_competitor(count, score, choice.physical.sides, &alternatives);
            if !report.ambiguity {
                retain(&mut report, choice, sides);
            }
            // Four-side ambiguity is not permission to fall back to a convenient inferred rectangle.
            break;
        }
    }
    Ok(report)
}
