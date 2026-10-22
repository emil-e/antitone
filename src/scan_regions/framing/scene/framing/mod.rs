//! Shared geometry and target-excluded peers for the Joint03 research detector.
use super::*;
use crate::{
    auto_crop,
    crop::Crop,
    scan_regions::{Areas, PictureGeometry, rectangles::project},
};

mod association;
mod choice;
mod geometry;
mod light;
mod measurement;
mod physical;
mod preparation;
#[cfg(test)]
#[path = "../../preparation_tests.rs"]
mod preparation_tests;
pub(in super::super) use preparation::run as prepare;
mod pooled;
#[cfg(test)]
mod refine;
#[cfg(test)]
mod tests;
use association::*;
use choice::Choice;
use geometry::visible_crop;
use measurement::*;

struct Prepared {
    roles: Vec<Vec<Role>>,
    base_votes: Vec<usize>,
}
impl Prepared {
    fn new(scene: &Scene, captures: &[Capture]) -> Self {
        Self::new_refs(scene, &captures.iter().collect::<Vec<_>>())
    }

    fn new_refs(scene: &Scene, captures: &[&Capture]) -> Self {
        let roles: Vec<_> = captures
            .iter()
            .map(|capture| scene.roles(capture))
            .collect();
        let mut base_votes = vec![0usize; scene.holder.len()];
        for role in &roles {
            for (count, &value) in base_votes.iter_mut().zip(role) {
                *count += usize::from(value == Role::BaseCompatible);
            }
        }
        Self { roles, base_votes }
    }
    fn persistent(&self, target: usize) -> Vec<bool> {
        let peers = self.roles.len().saturating_sub(1);
        self.base_votes
            .iter()
            .zip(&self.roles[target])
            .map(|(&count, &own)| {
                peers >= 2
                    && count - usize::from(own == Role::BaseCompatible)
                        >= 2.max((3 * peers).div_ceil(4))
            })
            .collect()
    }
}

fn observed_geometry(areas: &Areas) -> Option<PictureGeometry> {
    let mut frame = areas.picture_geometry.clone()?;
    for side in 0..4 {
        if !areas.picture_observed[side] || areas.picture_weak[side] {
            frame.sides[side] = None;
        }
    }
    Some(frame)
}

fn selected_lines(model: &RollModel, index: usize, capture: &Capture) -> [Option<Line>; 4] {
    let selection = model.select(index, capture);
    std::array::from_fn(|side| {
        selection
            .as_ref()
            .and_then(|selection| selection.family.as_ref())
            .and_then(|family| capture.selected_line(family, side))
            .cloned()
    })
}

fn nonfilm_fraction(capture: &Capture, roles: &[Role], crop: Crop) -> f64 {
    let mut invalid = 0;
    let mut samples = 0;
    for y in 0..8 {
        for x in 0..12 {
            let source = crop
                .source_point([
                    (f64::from(x) + 0.5) / 12. * f64::from(crop.size[0]),
                    (f64::from(y) + 0.5) / 8. * f64::from(crop.size[1]),
                ])
                .0;
            let raster = [
                (source[0] - f64::from(capture.area.x)) * f64::from(capture.size[0])
                    / f64::from(capture.area.width),
                (source[1] - f64::from(capture.area.y)) * f64::from(capture.size[1])
                    / f64::from(capture.area.height),
            ];
            if let Some((role, _)) = role_at(capture, roles, raster) {
                samples += 1;
                invalid += usize::from(matches!(role, Role::Holder | Role::BrighterThanBase));
            }
        }
    }
    invalid as f64 / samples.max(1) as f64
}

fn physical_ratio(frame: &PictureGeometry, active: SourceRegion) -> Option<f64> {
    let q = frame.frame(active)?;
    Some(
        (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]) / (q[3][0] - q[0][0]).hypot(q[3][1] - q[0][1]),
    )
}

#[derive(Serialize)]
struct Reference {
    ratio: Option<f64>,
    interval: Option<[f64; 2]>,
    #[serde(skip)]
    shape: Option<super::super::shape::Shape>,
    eligible: Vec<usize>,
    donors: Vec<usize>,
    rejected: Vec<usize>,
    error: Option<String>,
}

#[cfg(test)]
fn reference(
    scene: &Scene,
    captures: &[Capture],
    model: &RollModel,
    prepared: &Prepared,
    target: usize,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Reference, String> {
    reference_refs(
        scene,
        &captures.iter().collect::<Vec<_>>(),
        model,
        prepared,
        Some(target),
        checkpoint,
    )
}

fn reference_refs(
    scene: &Scene,
    captures: &[&Capture],
    model: &RollModel,
    prepared: &Prepared,
    target: Option<usize>,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Reference, String> {
    let mut donors = vec![];
    for (index, capture) in captures.iter().enumerate() {
        if target == Some(index) {
            continue;
        }
        checkpoint()?;
        let areas = model
            .areas(index, capture, capture.size, &capture.rgb)
            .map_err(|error| error.to_string())?;
        if auto_crop::propose(capture.area, &areas).is_err() {
            continue;
        }
        let Some(frame) = observed_geometry(&areas) else {
            continue;
        };
        let lines = selected_lines(model, index, capture);
        let support: [Support; 4] = std::array::from_fn(|side| {
            lines[side].as_ref().map_or_else(Support::default, |line| {
                measure(capture, &prepared.roles[index], &frame, side, line)
            })
        });
        if support
            .iter()
            .any(|value| value.contradicted() || value.kind() != Kind::Picture)
        {
            continue;
        }
        let limits = holder_limits(scene, frame.angle);
        let persistent = prepared.persistent(index);
        if crosses_separator(
            &frame,
            limits,
            &separators(capture, &prepared.roles[index], &persistent, frame.angle),
        ) {
            continue;
        }
        if let Some(ratio) = physical_ratio(&frame, capture.area) {
            donors.push((index, ratio));
        }
    }
    let eligible = donors.iter().map(|&(index, _)| index).collect();
    Ok(
        match super::super::shape::reference_fit(&donors, target.unwrap_or(usize::MAX)) {
            Ok(shape) => Reference {
                ratio: Some(shape.ratio),
                interval: Some(shape.interval),
                eligible,
                donors: shape.donors.clone(),
                rejected: shape.rejected.clone(),
                error: None,
                shape: Some(shape),
            },
            Err(reason) => Reference {
                ratio: None,
                interval: None,
                eligible,
                donors: vec![],
                rejected: vec![],
                error: Some(reason.into()),
                shape: None,
            },
        },
    )
}

fn candidate_locations(
    capture: &Capture,
    evidence: &Evidence,
    lines: &[Option<Line>; 4],
    pose: f64,
) -> [Vec<Option<(f64, Line)>>; 4] {
    std::array::from_fn(|side| {
        let mut rows: Vec<Option<(f64, Line)>> = vec![None];
        let mut native = evidence
            .lines
            .get(crate::scan_regions::lines::SIDES[side])
            .cloned()
            .unwrap_or_default();
        if let Some(line) = &lines[side] {
            native.push(line.clone());
        }
        for line in native {
            let midpoint = line.midpoint_pixel + 0.5;
            let point = if side < 2 {
                [midpoint, f64::from(capture.size[1]) / 2.]
            } else {
                [f64::from(capture.size[0]) / 2., midpoint]
            };
            let value = project(point, pose)[usize::from(side >= 2)];
            if !rows
                .iter()
                .flatten()
                .any(|(bound, _)| (*bound - value).abs() < 0.25)
            {
                rows.push(Some((value, line)));
            }
        }
        rows
    })
}

#[derive(Clone)]
struct Target<'a> {
    capture: &'a Capture,
    evidence: &'a Evidence,
    baseline: &'a Areas,
    lines: [Option<Line>; 4],
}
