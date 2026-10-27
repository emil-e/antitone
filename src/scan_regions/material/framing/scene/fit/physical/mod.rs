//! Roll-conditioned physical frame explanations, not arbitrary line assemblies.
use super::measurement::Pair;
use super::*;
use model::{Band, Exterior, Material, Template, median};
use neighbors::Neighbor;
use perimeter::Side;

#[cfg(test)]
mod anchor_tests;
#[cfg(test)]
mod color_tests;
#[cfg(test)]
pub(super) mod comparison;
#[cfg(test)]
mod comparison_tests;
#[cfg(test)]
mod coverage_tests;
#[cfg(test)]
pub(super) mod diagnostics;
mod directional;
#[cfg(test)]
mod directional_controls;
#[cfg(test)]
mod directional_tests;
#[cfg(test)]
mod geometry_tests;
mod joint;
#[cfg(test)]
mod joint_tests;
#[cfg(test)]
mod known_limitations;
#[cfg(test)]
mod limit_tests;
mod model;
mod neighbors;
mod perimeter;
pub(super) mod rigid;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod weak_fixtures;
#[cfg(test)]
mod weak_tests;

#[derive(Serialize)]
#[cfg(test)]
pub(super) struct Report {
    pub(super) proposal: Option<Choice>,
    template: Option<Template>,
    sides: Option<[Side; 4]>,
    neighbors: Vec<Neighbor>,
    neighbor_candidates: Vec<Neighbor>,
    pub(super) rejected: BTreeMap<String, usize>,
    pub(super) tested: usize,
    pub(super) ambiguity: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    joint_search: Option<joint::Search>,
}

#[cfg(test)]
pub(super) struct Context<'a> {
    pub(super) scene: &'a Scene,
    pub(super) captures: &'a [Capture],
    pub(super) model: &'a RollModel,
    pub(super) prepared: &'a Prepared,
    pub(super) reference: &'a Reference,
    pub(super) index: usize,
}

#[cfg(test)]
fn reject(report: &mut Report, reason: &str) {
    *report.rejected.entry(reason.into()).or_default() += 1;
}

#[derive(Clone, Copy)]
enum Mode {
    #[cfg(test)]
    Material,
    Joint,
}

impl Mode {
    fn joint_search(self) -> bool {
        matches!(self, Self::Joint)
    }
    fn measure(
        self,
        capture: &Capture,
        roles: &[Role],
        frame: &PictureGeometry,
        side: usize,
        exterior: Option<Exterior>,
    ) -> Side {
        match self {
            #[cfg(test)]
            Self::Material => perimeter::measure(capture, roles, frame, side, exterior),
            Self::Joint => directional::measure(capture, roles, frame, side, exterior),
        }
    }
    #[cfg(test)]
    fn method(self, count: usize, retained: bool) -> &'static str {
        match (self, count, retained) {
            (Self::Material, _, true) => "physical_retained_four",
            (Self::Material, 4, false) => "physical_four",
            (Self::Material, _, false) => "physical_three_plus_ratio",
            (Self::Joint, _, true) => "joint_retained_four",
            (Self::Joint, 4, false) => "joint_four",
            (Self::Joint, _, false) => "joint_three_plus_ratio",
        }
    }
}

// Complete available-picture qualification for material-backed calibration/retained checks.
// Nominal framing qualifies pose separately from material availability.
fn eligible_with(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    template: Option<&Template>,
    light: &light::ExteriorLight,
    mode: Mode,
) -> Result<(Crop, [Side; 4]), &'static str> {
    if template.is_some_and(|template| !template.compatible(frame)) {
        return Err("roll_geometry");
    }
    let bootstrap = scene.base.as_ref().map(|base| Exterior {
        mean: base.mean,
        spread: base.spread,
        samples: 0,
    });
    let sides = std::array::from_fn(|side| {
        mode.measure(
            capture,
            roles,
            frame,
            side,
            template.and_then(|t| t.exterior[side]).or(bootstrap),
        )
    });
    if sides.iter().any(|side| side.contradiction) {
        return Err("perimeter_contradiction");
    }
    let axes: Vec<_> = sides
        .iter()
        .filter(|side| side.neighbor_required)
        .filter_map(|side| side.neighbor.as_ref().map(|n| n.axis))
        .collect();
    if axes.iter().any(|axis| Some(axis) != axes.first()) {
        return Err("incompatible_neighbor_axes");
    }
    if light.intersects(frame) {
        return Err("physical_film_cut");
    }
    let visible = restricted(frame, holder_limits(scene, frame.angle));
    let Some(crop) = visible_crop(capture.area, &visible) else {
        return Err("no_visible_picture");
    };
    if nonfilm_fraction(capture, roles, crop) > 0.02 {
        return Err("visible_nonfilm");
    }
    Ok((crop, sides))
}

/// Qualify a nominal frame pose independently of how much film is available.
/// Nonfilm can censor an unmeasured side, but cannot supply a picture measurement.
fn nominal_with(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    template: Option<&Template>,
) -> Result<(Crop, [Side; 4]), &'static str> {
    if template.is_some_and(|template| !template.compatible(frame)) {
        return Err("roll_geometry");
    }
    let bootstrap = scene.base.as_ref().map(|base| Exterior {
        mean: base.mean,
        spread: base.spread,
        samples: 0,
    });
    let sides = std::array::from_fn(|side| {
        Mode::Joint.measure(
            capture,
            roles,
            frame,
            side,
            template.and_then(|t| t.exterior[side]).or(bootstrap),
        )
    });
    if sides.iter().any(|side| {
        side.directional
            .as_ref()
            .is_some_and(|report| report.contradiction)
    }) {
        return Err("perimeter_contradiction");
    }
    let axes: Vec<_> = sides
        .iter()
        .filter(|side| side.neighbor_required)
        .filter_map(|side| side.neighbor.as_ref().map(|n| n.axis))
        .collect();
    if axes.iter().any(|axis| Some(axis) != axes.first()) {
        return Err("incompatible_neighbor_axes");
    }
    let crop = visible_crop(capture.area, frame).ok_or("no_visible_picture")?;
    Ok((crop, sides))
}

pub(super) fn observe_native(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    baseline: &Areas,
) -> Option<PictureGeometry> {
    let frame = observed_geometry(baseline)?;
    if frame.sides.iter().all(Option::is_none) {
        return None;
    }
    // Perimeter stations need finite companion spans. Missing sides use only the
    // available camera envelope for assessment; they remain absent in the returned
    // observation and cannot become measured edges or dimension donors.
    let camera = [
        [0., 0.],
        [f64::from(capture.size[0]), 0.],
        capture.size.map(f64::from),
        [0., f64::from(capture.size[1])],
    ]
    .map(|p| project(p, frame.angle));
    let mut assessment = frame.clone();
    for (side, bound) in assessment.sides.iter_mut().enumerate() {
        if bound.is_none() {
            let values = camera.iter().map(|p| p[usize::from(side >= 2)]);
            *bound = Some(if side.is_multiple_of(2) {
                values.fold(f64::INFINITY, f64::min)
            } else {
                values.fold(f64::NEG_INFINITY, f64::max)
            });
        }
    }
    let (_, sides) = nominal_with(scene, capture, roles, &assessment, None).ok()?;
    let observed = PictureGeometry {
        sides: std::array::from_fn(|side| frame.sides[side].filter(|_| sides[side].measured())),
        ..frame
    };
    // A lone bright transition can be film termination rather than a picture edge.
    // Multiple credible sides tie the partial observation to a frame.
    (observed.sides.iter().filter(|side| side.is_some()).count() >= 2).then_some(observed)
}

/// Censoring is an inference from photo-compatible continuation at the camera limit,
/// never a measured edge or an observation from which to learn dimensions.
pub(super) fn camera_censoring(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
) -> [bool; 4] {
    std::array::from_fn(|side| {
        if frame.sides[side].is_some() {
            return false;
        }
        let mut positive = 0;
        let mut blocks = [false; 4];
        for station in 0..64 {
            let along =
                (station as f64 + 0.5) / 64. * f64::from(capture.size[usize::from(side < 2)]);
            let edge = if side.is_multiple_of(2) {
                0.5
            } else {
                f64::from(capture.size[usize::from(side >= 2)]) - 0.5
            };
            let point = if side < 2 {
                [edge, along]
            } else {
                [along, edge]
            };
            if !in_hypothesis(point, frame, None) {
                continue;
            }
            // A disconnected strip at the camera edge may be another frame/material;
            // edge darkness alone cannot establish continuation into this picture.
            let continuing = [0., 2., 4.].into_iter().all(|depth| {
                let mut probe = point;
                probe[usize::from(side >= 2)] += if side.is_multiple_of(2) {
                    depth
                } else {
                    -depth
                };
                in_hypothesis(probe, frame, None)
                    && role_at(capture, roles, probe)
                        .is_some_and(|(role, _)| role == Role::AttenuatedCompatible)
            });
            if continuing {
                positive += 1;
                blocks[station / 16] = true;
            }
        }
        // Bounded distributed support follows the directional station/block baseline;
        // this is not a calibrated probability that an unobserved boundary is outside.
        positive >= 12 && blocks.into_iter().filter(|v| *v).count() >= 3
    })
}

fn make_choice(
    frame: PictureGeometry,
    crop: Crop,
    sides: &[Side; 4],
    measured: [bool; 4],
    band: Option<[f64; 2]>,
    method: &'static str,
    capture: &Capture,
) -> Choice {
    let inferred = measured.map(|value| !value);
    let source_clipped = frame.frame(capture.area).is_none_or(|quad| {
        Areas::coverage(quad).map_or(true, |coverage| coverage.bounds(capture.area).is_err())
    });
    Choice {
        method,
        physical: frame,
        measured,
        inferred,
        crop,
        support: std::array::from_fn(|side| sides[side].support.clone()),
        pooled: std::array::from_fn(|side| sides[side].pooled.clone()),
        inferred_band_raster: band,
        source_clipped,
        outside_base_score: sides.iter().map(Side::score).sum(),
        signal_score: sides
            .iter()
            .filter_map(|side| side.pooled.as_ref())
            .map(|p| p.rank_signal)
            .sum(),
    }
}

#[cfg(test)]
fn distinct_competitor(
    count: usize,
    score: f64,
    bounds: [Option<f64>; 4],
    alternatives: &[(usize, f64, [Option<f64>; 4])],
) -> bool {
    alternatives.iter().any(|(n, s, other)| {
        *n == count
            && score - s < 0.1
            && (0..4).any(|side| (bounds[side].unwrap() - other[side].unwrap()).abs() > 3.)
    })
}

#[cfg(test)]
fn retain(report: &mut Report, choice: Choice, sides: [Side; 4]) {
    // A distant transition need not be a neighbor: it may be unresolved holder or other
    // exterior material. Only neighbors needed inside the assessed strip enter the model.
    report.neighbor_candidates = sides
        .iter()
        .filter_map(|side| side.neighbor.clone())
        .collect();
    report.neighbors = sides
        .iter()
        .filter(|side| side.neighbor_required)
        .filter_map(|side| side.neighbor.clone())
        .collect();
    report.sides = Some(sides);
    report.proposal = Some(choice);
}

#[cfg(test)]
fn candidates_with(
    capture: &Capture,
    roles: &[Role],
    target: &Target<'_>,
    template: &Template,
    pose: f64,
    mode: Mode,
) -> [Vec<Option<f64>>; 4] {
    ranked_locations(capture, roles, target, template, pose, mode).map(|ranked| {
        let mut result = vec![None];
        let limit = if mode.joint_search() { usize::MAX } else { 2 };
        result.extend(ranked.into_iter().take(limit).map(|(bound, _)| Some(bound)));
        result
    })
}

fn ranked_locations(
    capture: &Capture,
    roles: &[Role],
    target: &Target<'_>,
    template: &Template,
    pose: f64,
    mode: Mode,
) -> [Vec<(f64, f64)>; 4] {
    let prior = template.frame(pose);
    let native = candidate_locations(capture, target.evidence, &target.lines, pose);
    std::array::from_fn(|side| {
        let (center, radius) = if mode.joint_search() {
            let band = joint::search_band(template, pose, side);
            (band.center, band.radius)
        } else {
            (
                prior.sides[side].unwrap(),
                template.sides[side].radius
                    + f64::from(capture.size[0].max(capture.size[1]))
                        * (pose - template.pose.center).to_radians().abs(),
            )
        };
        let mut locations: Vec<f64> = native[side]
            .iter()
            .flatten()
            .map(|(bound, _)| *bound)
            .filter(|value| (*value - center).abs() <= radius)
            .take(8)
            .collect();
        let count = (2. * radius).ceil().min(32.) as usize;
        for index in 0..=count {
            let bound = (center - radius + 2. * radius * index as f64 / count.max(1) as f64 - 0.5)
                .round()
                + 0.5;
            if !locations.iter().any(|value| (*value - bound).abs() < 0.25) {
                locations.push(bound);
            }
        }
        let mut ranked = vec![];
        for value in locations {
            let mut trial = prior.clone();
            trial.sides[side] = Some(value);
            if mode.joint_search() {
                // A moved cue must not be scored against an inverted normal span.
                // This is a ranking probe at nominal size, not an accepted companion edge.
                let axis = usize::from(side >= 2);
                let extent = prior.sides[2 * axis + 1].unwrap() - prior.sides[2 * axis].unwrap();
                trial.sides[side ^ 1] = Some(
                    value
                        + if side.is_multiple_of(2) {
                            extent
                        } else {
                            -extent
                        },
                );
            }
            let support = mode.measure(capture, roles, &trial, side, template.exterior[side]);
            // Joint discovery ranks an uncertain cue; only completed companion
            // geometry can qualify it as an actual measured anchor.
            if support.measured() || mode.joint_search() {
                ranked.push((
                    value,
                    support.score() - (value - center).abs() / radius.max(1.) * 0.1,
                ));
            }
        }
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked
    })
}

#[cfg(test)]
pub(super) fn run(
    context: Context<'_>,
    target: Target<'_>,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Report, String> {
    run_with(context, target, checkpoint, Mode::Joint)
}

#[cfg(test)]
fn run_with(
    context: Context<'_>,
    target: Target<'_>,
    checkpoint: impl Fn() -> Result<(), String>,
    mode: Mode,
) -> Result<Report, String> {
    let template = fit_template(&context, &checkpoint)?;
    evaluate_with(
        context.scene,
        target,
        &context.prepared.roles[context.index],
        context.reference,
        template,
        checkpoint,
        mode,
    )
}

#[cfg(test)]
fn fit_template(
    context: &Context<'_>,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Option<Template>, String> {
    let mut frames = vec![];
    for &id in &context.reference.donors {
        if id == context.index {
            return Err("target cannot donate physical frame geometry".into());
        }
        checkpoint()?;
        let capture = &context.captures[id];
        let areas = context
            .model
            .areas(id, capture, capture.size, &capture.rgb)
            .map_err(|e| e.to_string())?;
        if let Some(frame) = observed_geometry(&areas)
            && frame.sides.iter().all(Option::is_some)
        {
            frames.push((id, frame));
        }
    }
    Ok(Template::fit(
        context.scene,
        context.captures,
        &context.prepared.roles,
        &frames,
    ))
}

#[cfg(test)]
fn evaluate(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    reference: &Reference,
    template: Option<Template>,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Report, String> {
    evaluate_with(
        scene,
        target,
        roles,
        reference,
        template,
        checkpoint,
        Mode::Joint,
    )
}

#[cfg(test)]
fn evaluate_with(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    reference: &Reference,
    template: Option<Template>,
    checkpoint: impl Fn() -> Result<(), String>,
    mode: Mode,
) -> Result<Report, String> {
    let mut report = Report {
        proposal: None,
        template,
        sides: None,
        neighbors: vec![],
        neighbor_candidates: vec![],
        rejected: BTreeMap::new(),
        tested: 0,
        ambiguity: false,
        joint_search: None,
    };
    let capture = target.capture;
    let light = light::ExteriorLight::new(capture, roles);
    // Established four-side geometry is kept only after the same physical/context checks;
    // it is never moved toward the peer ratio or an unrelated stronger line.
    if let Some(frame) = observed_geometry(target.baseline)
        && frame.sides.iter().all(Option::is_some)
    {
        match eligible_with(
            scene,
            capture,
            roles,
            &frame,
            report.template.as_ref(),
            &light,
            mode,
        ) {
            Ok((crop, sides)) if sides.iter().all(Side::measured) => {
                let choice = make_choice(
                    frame,
                    crop,
                    &sides,
                    [true; 4],
                    None,
                    mode.method(4, true),
                    capture,
                );
                retain(&mut report, choice, sides);
                return Ok(report);
            }
            Ok(_) => reject(&mut report, "unidentified_retained_anchor"),
            Err(reason) => reject(&mut report, reason),
        }
    }
    let Some(template) = report.template.clone() else {
        reject(&mut report, "insufficient_complete_peers");
        return Ok(report);
    };
    let pose = target
        .baseline
        .picture_geometry
        .as_ref()
        .map_or(template.pose.center, |frame| frame.angle);
    let pose = if template.pose.contains(pose) {
        pose
    } else {
        template.pose.center
    };
    if mode.joint_search() {
        return joint::evaluate(
            report,
            target,
            joint::Context {
                scene,
                roles,
                reference,
                template: &template,
                pose,
                mode,
            },
            checkpoint,
        );
    }
    let options = candidates_with(capture, roles, &target, &template, pose, mode);
    let mut best: Option<(usize, f64, Choice, [Side; 4])> = None;
    let mut alternatives = vec![];
    for l in &options[0] {
        for r in &options[1] {
            for t in &options[2] {
                for b in &options[3] {
                    let measured = [l.is_some(), r.is_some(), t.is_some(), b.is_some()];
                    let count = measured.iter().filter(|&&value| value).count();
                    if count < 3 {
                        continue;
                    }
                    report.tested += 1;
                    if report.tested.is_multiple_of(16) {
                        checkpoint()?;
                    }
                    let mut frame = PictureGeometry {
                        size: capture.size,
                        angle: pose,
                        sides: [*l, *r, *t, *b],
                    };
                    let band = if count == 3 {
                        let Some(shape) = &reference.shape else {
                            continue;
                        };
                        let Ok(draft) =
                            super::super::super::shape::complete(&frame, capture.area, shape)
                        else {
                            continue;
                        };
                        frame = draft.geometry;
                        Some(draft.inferred_band_raster)
                    } else {
                        None
                    };
                    let (crop, sides) = match eligible_with(
                        scene,
                        capture,
                        roles,
                        &frame,
                        Some(&template),
                        &light,
                        mode,
                    ) {
                        Ok(value) => value,
                        Err(reason) => {
                            reject(&mut report, reason);
                            continue;
                        }
                    };
                    if (0..4).any(|side| measured[side] && !sides[side].measured()) {
                        reject(&mut report, "unidentified_anchor");
                        continue;
                    }
                    let score =
                        sides.iter().map(Side::score).sum::<f64>() - template.displacement(&frame);
                    let choice = make_choice(
                        frame,
                        crop,
                        &sides,
                        measured,
                        band,
                        mode.method(count, false),
                        capture,
                    );
                    let wins = best
                        .as_ref()
                        .is_none_or(|(n, s, _, _)| count > *n || (count == *n && score > *s));
                    alternatives.push((count, score, choice.physical.sides));
                    if wins {
                        best = Some((count, score, choice, sides));
                    }
                }
            }
        }
    }
    if let Some((count, score, choice, sides)) = best {
        if distinct_competitor(count, score, choice.physical.sides, &alternatives) {
            report.ambiguity = true;
            reject(&mut report, "competing_physical_frames");
        } else {
            retain(&mut report, choice, sides);
        }
    }
    Ok(report)
}
