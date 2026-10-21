//! Fixed independent source dimensions; joint native posing with adjacent-anchor fallback.
use super::*;
use std::{cmp::Ordering, collections::BTreeSet};

mod calibration;
mod geometry;
mod joint;
mod projection;
#[cfg(test)]
mod tests;
pub(in super::super) use geometry::native_bound;
use geometry::{boundary_distance, frame_from_pair, validation_template};

// Exploratory equivalence settings, not calibrated blur widths or physical-error bounds.
const NEAR_SCORE: f64 = 0.1;
const NEAR_BOUNDARY: f64 = 3.;
const NATIVE_LIMIT: usize = 6;

#[derive(Clone, Serialize)]
pub(in super::super) struct Calibration {
    source_size: [f64; 2],
    donors: Vec<usize>,
    excluded_targets: Vec<usize>,
    template: Option<Template>,
}

#[derive(Serialize)]
pub(in super::super) struct CalibrationReport {
    pub(in super::super) model: Option<Calibration>,
    reference: Reference,
    donors: Vec<calibration::Donor>,
    unavailable: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Origin {
    NativePair,
    PriorPose,
}

#[derive(Clone, Serialize)]
struct Candidate {
    choice: Choice,
    anchor_score: f64,
    all_side_gradients: [projection::Projection; 4],
    actual_evidence_measured: [bool; 4],
    origin: Origin,
    native_angles: Option<[f64; 2]>,
}

#[derive(Serialize)]
struct Alternative {
    geometry: PictureGeometry,
    measured: [bool; 4],
    anchor_score: f64,
    boundary_distance: f64,
}

#[derive(Default, Serialize)]
pub(in super::super) struct Report {
    pub(in super::super) proposal: Option<Choice>,
    best: Option<Candidate>,
    ambiguity: bool,
    equivalent_near_ties: usize,
    distinct_near_ties: Vec<Alternative>,
    tested: usize,
    qualified: usize,
    native_qualified: usize,
    rejected: BTreeMap<String, usize>,
}

#[derive(Serialize)]
pub(in super::super) struct Outcome {
    pub(in super::super) proposal: Option<Choice>,
    used_joint: bool,
    joint_fit: joint::Report,
    two_edge: Option<Report>,
}

#[cfg(test)]
pub(in super::super) fn calibrate(
    captures: &[Capture],
    excluded_targets: &[usize],
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<CalibrationReport, String> {
    calibration::fit(captures, excluded_targets, checkpoint)
}

impl Outcome {
    pub(in super::super) fn ambiguous(&self) -> bool {
        self.two_edge
            .as_ref()
            .is_some_and(|report| report.ambiguity)
    }
}

pub(in super::super) fn group_model(
    captures: &[Capture],
    source_size: [f64; 2],
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Calibration, String> {
    let report = calibration::fit_group(captures, checkpoint)?;
    let mut model = report.model.unwrap_or(Calibration {
        source_size,
        donors: vec![],
        excluded_targets: vec![],
        template: None,
    });
    model.source_size = source_size;
    Ok(model)
}

pub(in super::super) fn run_group(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    calibration: &Calibration,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Outcome, String> {
    evaluate_optional(scene, target, roles, calibration, checkpoint)
}

#[cfg(test)]
pub(in super::super) fn run(
    context: &Context<'_>,
    target: Target<'_>,
    calibration: &Calibration,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Outcome, String> {
    if !calibration.excluded_targets.contains(&context.index)
        || calibration.donors.contains(&context.index)
    {
        return Err("rigid target was not excluded from calibration".into());
    }
    evaluate_optional(
        context.scene,
        target,
        &context.prepared.roles[context.index],
        calibration,
        checkpoint,
    )
}

fn evaluate_optional(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    calibration: &Calibration,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Outcome, String> {
    let joint_fit = joint::evaluate(scene, &target, roles, calibration, &checkpoint)?;
    if let Some(proposal) = joint_fit.proposal.clone() {
        return Ok(Outcome {
            proposal: Some(proposal),
            used_joint: true,
            joint_fit,
            two_edge: None,
        });
    }
    let two_edge = search(scene, target, roles, calibration, &checkpoint)?;
    Ok(Outcome {
        proposal: two_edge.proposal.clone(),
        used_joint: false,
        joint_fit,
        two_edge: Some(two_edge),
    })
}

fn assess(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    calibration: &Calibration,
    frame: PictureGeometry,
    measured: [bool; 4],
    light: &light::ExteriorLight,
) -> Result<Candidate, &'static str> {
    if measured[..2].iter().filter(|&&v| v).count() != 1
        || measured[2..].iter().filter(|&&v| v).count() != 1
    {
        return Err("nonadjacent_anchors");
    }
    let template = validation_template(capture, calibration, &frame);
    let (crop, sides) = eligible_with(
        scene,
        capture,
        roles,
        &frame,
        template.as_ref(),
        light,
        Mode::Joint,
    )?;
    let actual = sides.each_ref().map(Side::measured);
    if (0..4).any(|side| measured[side] && !actual[side]) {
        return Err("unidentified_completed_anchor");
    }
    let anchor_score = (0..4)
        .filter(|&side| measured[side])
        .map(|side| sides[side].score())
        .sum();
    let all_side_gradients =
        std::array::from_fn(|side| projection::measure(capture, &frame, side, &sides[side]));
    Ok(Candidate {
        choice: make_choice(
            frame,
            crop,
            &sides,
            measured,
            None,
            "rigid_two_adjacent",
            capture,
        ),
        anchor_score,
        all_side_gradients,
        actual_evidence_measured: actual,
        origin: Origin::PriorPose,
        native_angles: None,
    })
}

fn compare(a: &Candidate, b: &Candidate) -> Ordering {
    a.anchor_score
        .total_cmp(&b.anchor_score)
        .then_with(|| {
            let total = |candidate: &Candidate| {
                candidate
                    .all_side_gradients
                    .iter()
                    .map(|v| v.mean)
                    .sum::<f64>()
            };
            total(a).total_cmp(&total(b))
        })
        .then_with(|| (a.origin == Origin::NativePair).cmp(&(b.origin == Origin::NativePair)))
        .then_with(|| b.choice.physical.angle.total_cmp(&a.choice.physical.angle))
        .then_with(|| {
            (0..4)
                .map(|side| {
                    b.choice.physical.sides[side]
                        .unwrap()
                        .total_cmp(&a.choice.physical.sides[side].unwrap())
                })
                .find(|order| *order != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| a.choice.measured.cmp(&b.choice.measured))
}

fn select(report: &mut Report, candidates: Vec<Candidate>) {
    report.qualified = candidates.len();
    report.native_qualified = candidates
        .iter()
        .filter(|v| v.origin == Origin::NativePair)
        .count();
    let Some(best) = candidates.iter().max_by(|a, b| compare(a, b)) else {
        return;
    };
    for other in &candidates {
        if std::ptr::eq(best, other) || best.anchor_score - other.anchor_score >= NEAR_SCORE {
            continue;
        }
        let distance = boundary_distance(&best.choice.physical, &other.choice.physical);
        if distance > NEAR_BOUNDARY {
            report.distinct_near_ties.push(Alternative {
                geometry: other.choice.physical.clone(),
                measured: other.choice.measured,
                anchor_score: other.anchor_score,
                boundary_distance: distance,
            });
        } else {
            report.equivalent_near_ties += 1;
        }
    }
    report.ambiguity = !report.distinct_near_ties.is_empty();
    if !report.ambiguity {
        report.proposal = Some(best.choice.clone());
    }
    report.best = Some(best.clone());
}

fn search(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    calibration: &Calibration,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Report, String> {
    let capture = target.capture;
    let mut lines: [Vec<Line>; 4] = std::array::from_fn(|side| {
        let mut values = target
            .evidence
            .lines
            .get(crate::scan_regions::lines::SIDES[side])
            .cloned()
            .unwrap_or_default();
        values.extend(target.lines[side].iter().cloned());
        values.retain(|line| {
            !line.angle_limit_reached
                && line.fraction() > 0.
                && line.angle_degrees.is_finite()
                && line.midpoint_pixel.is_finite()
        });
        values.sort_by(|a, b| {
            a.midpoint_pixel
                .total_cmp(&b.midpoint_pixel)
                .then_with(|| a.angle_degrees.total_cmp(&b.angle_degrees))
                .then_with(|| {
                    b.lower_quartile_gradient_score
                        .total_cmp(&a.lower_quartile_gradient_score)
                })
        });
        values.dedup_by(|a, b| {
            a.midpoint_pixel == b.midpoint_pixel && a.angle_degrees == b.angle_degrees
        });
        values.sort_by(|a, b| {
            b.lower_quartile_gradient_score
                .total_cmp(&a.lower_quartile_gradient_score)
                .then_with(|| a.midpoint_pixel.total_cmp(&b.midpoint_pixel))
                .then_with(|| a.angle_degrees.total_cmp(&b.angle_degrees))
        });
        values.truncate(NATIVE_LIMIT);
        values
    });
    for side in &mut lines {
        side.sort_by(|a, b| {
            a.midpoint_pixel
                .total_cmp(&b.midpoint_pixel)
                .then_with(|| a.angle_degrees.total_cmp(&b.angle_degrees))
        });
    }
    let light = light::ExteriorLight::new(capture, roles);
    let mut report = Report::default();
    let mut candidates = vec![];
    let mut seen = BTreeSet::new();
    let mut trial = |frame: PictureGeometry, measured: [bool; 4], angles: Option<[f64; 2]>| {
        let key = (
            frame.angle.to_bits(),
            frame.sides.map(|v| v.unwrap().to_bits()),
            measured,
        );
        if !seen.insert(key) {
            return;
        }
        report.tested += 1;
        match assess(scene, capture, roles, calibration, frame, measured, &light) {
            Ok(mut candidate) => {
                candidate.native_angles = angles;
                candidate.origin = if angles.is_some() {
                    Origin::NativePair
                } else {
                    Origin::PriorPose
                };
                candidates.push(candidate);
            }
            Err(reason) => {
                *report.rejected.entry(reason.into()).or_default() += 1;
            }
        }
    };
    for vertical in 0..2 {
        for horizontal in 2..4 {
            for vline in &lines[vertical] {
                for hline in &lines[horizontal] {
                    checkpoint()?;
                    let angles = [
                        vline.physical_angle(vertical),
                        hline.physical_angle(horizontal),
                    ];
                    let pose = (angles[0] + angles[1]) * 0.5;
                    if let (Some(v), Some(h)) = (
                        native_bound(vline, vertical, capture.size, pose),
                        native_bound(hline, horizontal, capture.size, pose),
                    ) && let Some(frame) = frame_from_pair(
                        capture,
                        calibration.source_size,
                        vertical,
                        horizontal,
                        [v, h],
                        pose,
                    ) {
                        trial(
                            frame,
                            std::array::from_fn(|i| i == vertical || i == horizontal),
                            Some(angles),
                        );
                    }
                }
            }
        }
    }
    let Some(prior) = &calibration.template else {
        select(&mut report, candidates);
        return Ok(report);
    };
    let pose = target
        .baseline
        .picture_geometry
        .as_ref()
        .map_or(prior.pose.center, |frame| frame.angle);
    let pose = if prior.pose.contains(pose) {
        pose
    } else {
        prior.pose.center
    };
    let locations = ranked_locations(capture, roles, &target, prior, pose, Mode::Joint);
    for vertical in 0..2 {
        for horizontal in 2..4 {
            for &(v, _) in &locations[vertical] {
                for &(h, _) in &locations[horizontal] {
                    checkpoint()?;
                    if let Some(frame) = frame_from_pair(
                        capture,
                        calibration.source_size,
                        vertical,
                        horizontal,
                        [v, h],
                        pose,
                    ) {
                        trial(
                            frame,
                            std::array::from_fn(|i| i == vertical || i == horizontal),
                            None,
                        );
                    }
                }
            }
        }
    }
    select(&mut report, candidates);
    Ok(report)
}
