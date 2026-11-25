//! Source-owned dust-repair instructions and donor search.

use crate::{Error, geometry::SourceRegion};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
pub(crate) struct Execution<'a> {
    pub cancelled: &'a dyn Fn() -> bool,
}

fn never_cancel() -> bool {
    false
}

impl Default for Execution<'_> {
    fn default() -> Self {
        Self {
            cancelled: &never_cancel,
        }
    }
}

pub type Ap1 = [f32; 3];
pub(crate) const CONTEXT_PADDING: u32 = 10;
#[allow(dead_code)]
pub(crate) const SEARCH_ACCEPTABLE_SCORE: f64 = 0.02;
// Grain makes small context-score differences poor evidence for moving farther.
// Prefer spatial coherence among comparable matches, not the lowest noisy score.
const SEARCH_COMPETITIVE_FRACTION: f64 = 0.10;
#[allow(dead_code)]
pub(crate) const SEARCH_BUDGET: Duration = Duration::from_millis(300);
pub(crate) const AUTOMATIC_PADDING: u32 = 128 + 4;

mod base_image;
pub(crate) mod context;
mod matching;
mod support;
pub use crate::photographic::stages::healing::{EvaluatedRepair, evaluate, heal_ranked};
pub(crate) use base_image::BaseImage;
pub use context::DonorMatch;
use context::{NearSupport, SearchTarget, build_search_target};
#[cfg(all(test, feature = "editor"))]
pub(crate) use support::{MAX_MASK_PIXELS, MAX_MASK_WORK_PIXELS};
pub use support::{
    SupportMask, donor_target, initial_offset, mask_within_budget, nearest_offset, offset_target,
    validate_offset,
};

pub const REPAIR_CONTRACT: &str = "ungraded-ap1-boundary-ranked-offset-v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
pub struct RepairId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(rename_all = "snake_case", deny_unknown_fields)
)]
pub enum RepairOrigin {
    Manual,
    Detected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Repair {
    pub id: RepairId,
    pub enabled: bool,
    pub origin: RepairOrigin,
    pub support: SupportMask,
    /// One donor translation for the whole repair, shared by every component.
    /// Stored, not recomputed: healing is deterministic from it.
    pub offset: [i32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct RepairSet {
    contract: String,
    last_id: u64,
    repairs: Vec<Repair>,
}

impl Default for RepairSet {
    fn default() -> Self {
        Self {
            contract: REPAIR_CONTRACT.into(),
            last_id: 0,
            repairs: Vec::new(),
        }
    }
}

impl RepairSet {
    pub fn repairs(&self) -> &[Repair] {
        &self.repairs
    }

    pub fn validate(&self, active: SourceRegion) -> Result<(), Error> {
        if self.contract != REPAIR_CONTRACT {
            return Err(Error::InvalidParameter("unsupported repair contract"));
        }

        active.validate()?;
        let mut ids = std::collections::HashSet::new();
        for repair in &self.repairs {
            if repair.id.0 == 0 || repair.id.0 > self.last_id || !ids.insert(repair.id) {
                return Err(Error::InvalidParameter("invalid repair ID allocator"));
            }
            repair.support.validate()?;
            let [x, y] = repair.support.origin();
            let [width, height] = repair.support.size();
            if !active.contains_region(SourceRegion {
                x,
                y,
                width,
                height,
            }) {
                return Err(Error::InvalidParameter(
                    "repair support is outside the source",
                ));
            }
            validate_offset(repair.offset, &repair.support, active)?;
        }

        for (index, repair) in self.repairs.iter().enumerate() {
            if self.repairs[..index]
                .iter()
                .any(|other| other.support.intersects(&repair.support))
            {
                return Err(Error::InvalidParameter("repair supports overlap"));
            }
        }
        Ok(())
    }

    /// Reset accepted repairs without reusing any previously allocated identity.
    pub fn clear(&mut self) {
        self.repairs.clear();
    }

    pub fn insert(
        &mut self,
        support: SupportMask,
        origin: RepairOrigin,
        offset: [i32; 2],
    ) -> Result<RepairId, Error> {
        self.require_independent(&support, None)?;
        let next = self
            .last_id
            .checked_add(1)
            .ok_or(Error::InvalidParameter("repair ID space exhausted"))?;
        let id = RepairId(next);
        self.last_id = next;
        self.repairs.push(Repair {
            id,
            enabled: true,
            origin,
            support,
            offset,
        });
        Ok(id)
    }

    pub fn remove(&mut self, id: RepairId) -> Result<Repair, Error> {
        let index = self
            .repairs
            .iter()
            .position(|repair| repair.id == id)
            .ok_or(Error::InvalidParameter("unknown repair ID"))?;
        Ok(self.repairs.remove(index))
    }

    pub fn set_enabled(&mut self, id: RepairId, enabled: bool) -> Result<(), Error> {
        self.repair_mut(id)?.enabled = enabled;
        Ok(())
    }

    pub fn set_offset(&mut self, id: RepairId, offset: [i32; 2]) -> Result<(), Error> {
        self.repair_mut(id)?.offset = offset;
        Ok(())
    }

    pub fn set_support(&mut self, id: RepairId, support: SupportMask) -> Result<(), Error> {
        self.require_independent(&support, Some(id))?;
        self.repair_mut(id)?.support = support;
        Ok(())
    }

    fn repair_mut(&mut self, id: RepairId) -> Result<&mut Repair, Error> {
        self.repairs
            .iter_mut()
            .find(|repair| repair.id == id)
            .ok_or(Error::InvalidParameter("unknown repair ID"))
    }

    fn require_independent(
        &self,
        support: &SupportMask,
        except: Option<RepairId>,
    ) -> Result<(), Error> {
        if self
            .repairs
            .iter()
            .any(|repair| Some(repair.id) != except && repair.support.intersects(support))
        {
            return Err(Error::InvalidParameter("repair supports overlap"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchOutcome {
    Candidate([i32; 2]),
    Impossible,
    Cancelled,
}

/// Geometry establishes the search's own last-resort candidate. Coarse samples
/// cover a useful neighbourhood before refining promising positions. The time
/// ceiling includes pixel acquisition and scoring, not just candidate admission.
#[allow(dead_code)]
pub(crate) fn search_donor(
    support: &SupportMask,
    preferred: Option<[i32; 2]>,
    active: SourceRegion,
    base: &mut impl FnMut([u32; 2]) -> Result<Ap1, Error>,
    deadline: Instant,
    threshold: f64,
    execution: &mut Execution<'_>,
) -> Result<SearchOutcome, Error> {
    search_image(
        support, preferred, active, base, deadline, threshold, execution,
    )
}

pub(crate) fn search_image(
    support: &SupportMask,
    preferred: Option<[i32; 2]>,
    active: SourceRegion,
    base: &mut impl BaseImage,
    deadline: Instant,
    threshold: f64,
    execution: &mut Execution<'_>,
) -> Result<SearchOutcome, Error> {
    if (execution.cancelled)() {
        return Ok(SearchOutcome::Cancelled);
    }
    active.validate()?;
    support.validate()?;
    let [x, y] = support.origin();
    let [width, height] = support.size();
    if !active.contains_region(SourceRegion {
        x,
        y,
        width,
        height,
    }) {
        return Err(Error::InvalidParameter(
            "repair support is outside the source",
        ));
    }
    let fallback = preferred
        .filter(|offset| validate_offset(*offset, support, active).is_ok())
        .or_else(|| initial_offset(support, active));
    if (execution.cancelled)() {
        return Ok(SearchOutcome::Cancelled);
    }
    let Some(fallback) = fallback else {
        return Ok(SearchOutcome::Impossible);
    };
    if Instant::now() >= deadline {
        return Ok(SearchOutcome::Candidate(fallback));
    }
    let stopped = || (execution.cancelled)() || Instant::now() >= deadline;
    let mut bounded = Execution {
        cancelled: &stopped,
    };
    let broad_deadline =
        Instant::now() + deadline.saturating_duration_since(Instant::now()) * 2 / 3;
    let mut targets = Vec::new();
    for group in support.grouped_supports() {
        if let Some(target) = build_search_target(support, &group, active, base, &mut bounded)? {
            targets.push(target);
        }
        if stopped() {
            break;
        }
    }
    if stopped() || targets.is_empty() {
        return Ok(if (execution.cancelled)() {
            SearchOutcome::Cancelled
        } else {
            SearchOutcome::Candidate(fallback)
        });
    }
    let Some(matcher) = matching::Matcher::new(&targets, &stopped) else {
        return Ok(if (execution.cancelled)() {
            SearchOutcome::Cancelled
        } else {
            SearchOutcome::Candidate(fallback)
        });
    };
    let near = NearSupport::new(support);
    let footprint = support::RequiredFootprint::new(support);
    let target = donor_target(support, active)?;
    let minimum = [
        active.x as i32 - target.x as i32,
        active.y as i32 - target.y as i32,
    ];
    let maximum = [
        (active.x + active.width - target.x - target.width) as i32,
        (active.y + active.height - target.y - target.height) as i32,
    ];
    let mut seen = std::collections::HashSet::new();
    let mut consider =
        |offset: [i32; 2], candidates: &mut Vec<(f64, [i32; 2])>| -> Result<bool, Error> {
            if stopped() {
                return Ok(false);
            }
            if !(0..2).all(|axis| offset[axis] >= minimum[axis] && offset[axis] <= maximum[axis])
                || !seen.insert(offset)
                || !footprint.clear(support, offset)
            {
                return Ok(true);
            }
            let Some(score) = matcher.score(support, &near, offset, base, &mut bounded)? else {
                return Ok(false);
            };
            candidates.push((score, offset));
            Ok(score > threshold)
        };
    let mut candidates = Vec::new();
    if !consider(fallback, &mut candidates)? {
        return Ok(if (execution.cancelled)() {
            SearchOutcome::Cancelled
        } else {
            SearchOutcome::Candidate(fallback)
        });
    }
    let span = support.size.iter().copied().max().unwrap() as i32 + 2;
    let mut local = Vec::new();
    let local_deadline = Instant::now() + Duration::from_millis(30);
    let local_stopped = || stopped() || Instant::now() >= local_deadline;
    let mut valid = 0usize;
    support::visit_offsets(support, active, &local_stopped, |offset| {
        valid += 1;
        if valid <= 64 || valid.is_multiple_of(8) {
            local.push(offset);
        }
        local.len() < 256
    });
    let mut schedule = vec![fallback];
    schedule.extend(matcher.proposals(support, &stopped));
    // Probe both close donors and separated directions before spending the rest
    // on a spatial grid. A long stroke can need a translation larger than 128.
    for radius in [span, 8, 16, 32, 64, 96, 128] {
        for (dx, dy) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
        ] {
            schedule.push([dx * radius, dy * radius]);
        }
    }
    // A coarse grid reaches beyond the nearest rings. Bit-reversed coordinates
    // avoid spending successive samples on adjacent cells.
    let extent = span.max(128);
    for index in 0..256u32 {
        if let Some(offset) = local.get(index as usize) {
            schedule.push(*offset);
        }
        let x = (index.reverse_bits() >> 28) as i32;
        let y = ((index >> 4).reverse_bits() >> 28) as i32;
        schedule.push([(2 * x - 15) * extent / 15, (2 * y - 15) * extent / 15]);
    }
    let mut finished = false;
    for offset in schedule {
        if Instant::now() >= broad_deadline && !candidates.is_empty() {
            break;
        }
        if !consider(offset, &mut candidates)? {
            finished = true;
            break;
        }
    }
    if !finished {
        let mut ranked = candidates.clone();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, seed) in ranked.iter().take(3) {
            let mut center = seed;
            for step in [16, 8, 4, 2, 1] {
                for (dx, dy) in [
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ] {
                    if !consider(
                        [center[0] + dx * step, center[1] + dy * step],
                        &mut candidates,
                    )? {
                        finished = true;
                        break;
                    }
                }
                if finished {
                    break;
                }
                if let Some(&(_, best)) = candidates.iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
                    center = best;
                }
            }
            if finished {
                break;
            }
        }
    }
    if (execution.cancelled)() {
        return Ok(SearchOutcome::Cancelled);
    }
    let best = candidates
        .iter()
        .map(|(score, _)| *score)
        .fold(f64::INFINITY, f64::min);
    if !best.is_finite() {
        return Ok(SearchOutcome::Candidate(fallback));
    }
    let margin = competitive_score(best);
    let selected = candidates
        .iter()
        .filter(|(score, _)| *score <= margin)
        .min_by(|a, b| {
            offset_distance(a.1)
                .cmp(&offset_distance(b.1))
                .then_with(|| a.0.total_cmp(&b.0))
        })
        .map(|(_, offset)| *offset);
    Ok(SearchOutcome::Candidate(selected.unwrap_or(fallback)))
}

fn offset_distance(offset: [i32; 2]) -> u64 {
    let x = u64::from(offset[0].unsigned_abs());
    let y = u64::from(offset[1].unsigned_abs());
    x * x + y * y
}

fn competitive_score(best: f64) -> f64 {
    best * (1. + SEARCH_COMPETITIVE_FRACTION) + 1e-12
}

fn distance(x: usize, y: usize, tx: usize, ty: usize) -> f64 {
    ((x as f64 - tx as f64).powi(2) + (y as f64 - ty as f64).powi(2)).sqrt()
}

#[allow(dead_code)]
fn selected_bounds(mask: &[bool], width: usize) -> Option<[usize; 4]> {
    let mut bounds = [usize::MAX, usize::MAX, 0, 0];
    let mut any = false;
    for (i, selected) in mask.iter().enumerate() {
        if *selected {
            let (x, y) = (i % width, i / width);
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x + 1);
            bounds[3] = bounds[3].max(y + 1);
            any = true;
        }
    }
    any.then_some(bounds)
}

fn grouped_components(mask: &[bool], width: usize, height: usize) -> Vec<Vec<usize>> {
    let expanded = dilate(mask, width, height, 1);
    let mut labels = vec![usize::MAX; mask.len()];
    let mut groups = Vec::new();
    for start in 0..expanded.len() {
        if !expanded[start] || labels[start] != usize::MAX {
            continue;
        }
        let label = groups.len();
        let mut queue = VecDeque::from([start]);
        labels[start] = label;
        while let Some(index) = queue.pop_front() {
            let x = index % width;
            let y = index / width;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let next = ny * width + nx;
                    if expanded[next] && labels[next] == usize::MAX {
                        labels[next] = label;
                        queue.push_back(next);
                    }
                }
            }
        }
        groups.push(Vec::new());
    }
    for (index, hidden) in mask.iter().enumerate() {
        if *hidden {
            groups[labels[index]].push(index);
        }
    }
    groups
        .into_iter()
        .filter(|group| !group.is_empty())
        .collect()
}

fn dilate(mask: &[bool], width: usize, height: usize, radius: usize) -> Vec<bool> {
    let mut output = vec![false; mask.len()];
    for y in 0..height {
        for x in 0..width {
            output[y * width + x] =
                (y.saturating_sub(radius)..=(y + radius).min(height - 1)).any(|ny| {
                    (x.saturating_sub(radius)..=(x + radius).min(width - 1))
                        .any(|nx| mask[ny * width + nx])
                });
        }
    }
    output
}

#[cfg(test)]
mod matching_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
