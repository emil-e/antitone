//! Source-owned dust-repair instructions and versioned AP1 donor healer.
//!
//! Repair support is binary and source-coordinate owned. Manual and detected
//! supports differ only in UI provenance; both enter [`heal_ranked`] identically.
use crate::{Error, geometry::SourceRegion};
use std::collections::VecDeque;
mod screening;
pub(crate) use screening::Execution;
#[cfg(feature = "editor")]
pub(crate) use screening::{ScreenInput, Screener};

pub type Ap1 = [f32; 3];
pub(crate) const CONTEXT_PADDING: u32 = 10;
pub(crate) const AUTOMATIC_PADDING: u32 = 128 + 4;

mod support;
#[cfg(all(test, feature = "editor"))]
pub(crate) use support::{MAX_MASK_PIXELS, MAX_MASK_WORK_PIXELS};
pub use support::{SupportMask, donor_target, mask_within_budget, validate_donor};

pub const REPAIR_CONTRACT: &str = "ungraded-ap1-boundary-ranked-score10-grow4-v1";

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(rename_all = "snake_case", deny_unknown_fields)
)]
pub enum DonorChoice {
    #[default]
    Automatic,
    Offset([i32; 2]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Repair {
    pub id: RepairId,
    pub enabled: bool,
    pub origin: RepairOrigin,
    pub support: SupportMask,
    pub donor: DonorChoice,
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
            validate_donor(repair.donor, &repair.support, active)?;
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
        donor: DonorChoice,
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
            donor,
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

    pub fn set_donor(&mut self, id: RepairId, donor: DonorChoice) -> Result<(), Error> {
        self.repair_mut(id)?.donor = donor;
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

#[derive(Clone, Debug, PartialEq)]
pub struct DonorMatch {
    pub target: [u32; 4],
    pub source: [u32; 2],
    pub context_mse: f64,
    pub paired_context_pixels: usize,
    pub matching_score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedRepair {
    pub id: RepairId,
    support: SupportMask,
    pub donors: Vec<DonorMatch>,
    values: Vec<Ap1>,
}

impl EvaluatedRepair {
    /// Explicit immutable result payload, excluding allocator bookkeeping and shared ownership.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.support.retained_bytes()
            - std::mem::size_of::<SupportMask>()
            + self.values.capacity() * std::mem::size_of::<Ap1>()
            + self.donors.capacity() * std::mem::size_of::<DonorMatch>()
    }

    pub fn support(&self) -> &SupportMask {
        &self.support
    }

    pub fn pixels(&self) -> impl Iterator<Item = ([u32; 2], Ap1)> + '_ {
        self.support
            .selected_points()
            .map(|point| (point, self.pixel(point, [0.; 3])))
    }

    pub fn pixel(&self, point: [u32; 2], base: Ap1) -> Ap1 {
        if !self.support.contains(point) {
            return base;
        }
        let index = ((point[1] - self.support.origin[1]) * self.support.size[0] + point[0]
            - self.support.origin[0]) as usize;
        self.values[index]
    }
}

/// Evaluates one repair against immutable base AP1. Existing evaluated repairs
/// are not inputs, so changing this repair cannot alter any other repair.
pub fn evaluate(
    repair: &Repair,
    active: SourceRegion,
    base: impl FnMut([u32; 2]) -> Result<Ap1, Error>,
) -> Result<EvaluatedRepair, Error> {
    evaluate_with(repair, active, base, &mut Execution::default())
        .map(|result| result.expect("uncancelled repair evaluation"))
}

pub(crate) fn evaluate_with(
    repair: &Repair,
    active: SourceRegion,
    mut base: impl FnMut([u32; 2]) -> Result<Ap1, Error>,
    execution: &mut Execution<'_>,
) -> Result<Option<EvaluatedRepair>, Error> {
    if (execution.cancelled)() {
        return Ok(None);
    }
    active.validate()?;
    let support_right = repair.support.origin[0]
        .checked_add(repair.support.size[0])
        .ok_or(Error::InvalidParameter(
            "repair support coordinates overflow",
        ))?;
    let support_bottom = repair.support.origin[1]
        .checked_add(repair.support.size[1])
        .ok_or(Error::InvalidParameter(
            "repair support coordinates overflow",
        ))?;
    if repair.support.origin[0] < active.x
        || repair.support.origin[1] < active.y
        || support_right > active.x + active.width
        || support_bottom > active.y + active.height
    {
        return Err(Error::InvalidParameter(
            "repair support is outside the source",
        ));
    }
    repair.support.validate()?;
    let selected = repair.support.selected_indices().collect::<Vec<_>>();
    let local_width = repair.support.size[0] as usize;
    let min_x = repair.support.origin[0]
        + selected
            .iter()
            .map(|index| index % local_width)
            .min()
            .unwrap() as u32;
    let max_x = repair.support.origin[0]
        + selected
            .iter()
            .map(|index| index % local_width)
            .max()
            .unwrap() as u32;
    let min_y = repair.support.origin[1]
        + selected
            .iter()
            .map(|index| index / local_width)
            .min()
            .unwrap() as u32;
    let max_y = repair.support.origin[1]
        + selected
            .iter()
            .map(|index| index / local_width)
            .max()
            .unwrap() as u32;
    let target = SourceRegion {
        x: min_x.saturating_sub(CONTEXT_PADDING).max(active.x),
        y: min_y.saturating_sub(CONTEXT_PADDING).max(active.y),
        width: 0,
        height: 0,
    };
    let target_right = max_x
        .saturating_add(CONTEXT_PADDING + 1)
        .min(active.x + active.width);
    let target_bottom = max_y
        .saturating_add(CONTEXT_PADDING + 1)
        .min(active.y + active.height);
    let target = SourceRegion {
        width: target_right - target.x,
        height: target_bottom - target.y,
        ..target
    };
    let window = match repair.donor {
        DonorChoice::Automatic => padded(target, active, AUTOMATIC_PADDING),
        DonorChoice::Offset([dx, dy]) => {
            let source_x = offset(target.x as usize, dx)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(Error::InvalidParameter("manual donor outside source"))?;
            let source_y = offset(target.y as usize, dy)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(Error::InvalidParameter("manual donor outside source"))?;
            let source = SourceRegion {
                x: source_x,
                y: source_y,
                width: target.width,
                height: target.height,
            };
            if source.x < active.x
                || source.y < active.y
                || source.x.checked_add(source.width) > Some(active.x + active.width)
                || source.y.checked_add(source.height) > Some(active.y + active.height)
            {
                return Err(Error::InvalidParameter("manual donor outside source"));
            }
            let union = SourceRegion {
                x: target.x.min(source.x),
                y: target.y.min(source.y),
                width: target_right.max(source.x + source.width) - target.x.min(source.x),
                height: target_bottom.max(source.y + source.height) - target.y.min(source.y),
            };
            padded(union, active, 4)
        }
    };
    let count = window
        .width
        .checked_mul(window.height)
        .filter(|count| *count <= 4_000_000)
        .ok_or(Error::InvalidParameter(
            "repair evaluation window is too large",
        ))? as usize;
    let mut image = Vec::with_capacity(count);
    for y in window.y..window.y + window.height {
        if (execution.cancelled)() {
            return Ok(None);
        }
        for x in window.x..window.x + window.width {
            image.push(base([x, y])?);
        }
    }
    let mut mask = vec![false; count];
    for index in selected {
        let x = repair.support.origin[0] + (index % local_width) as u32;
        let y = repair.support.origin[1] + (index / local_width) as u32;
        mask[((y - window.y) * window.width + x - window.x) as usize] = true;
    }
    let Some((result, mut donors)) = heal_with_execution(
        &image,
        [window.width, window.height],
        &mask,
        repair.donor,
        DonorSearch::Bounded,
        execution,
    )?
    else {
        return Ok(None);
    };
    for donor in &mut donors {
        donor.target[0] += window.x;
        donor.target[1] += window.y;
        donor.source[0] += window.x;
        donor.source[1] += window.y;
    }
    let mut values = vec![[0.; 3]; (repair.support.size[0] * repair.support.size[1]) as usize];
    for index in repair.support.selected_indices() {
        let x = repair.support.origin[0] + (index % local_width) as u32;
        let y = repair.support.origin[1] + (index / local_width) as u32;
        values[index] = result[((y - window.y) * window.width + x - window.x) as usize];
    }
    if (execution.cancelled)() {
        return Ok(None);
    }
    Ok(Some(EvaluatedRepair {
        id: repair.id,
        support: repair.support.clone(),
        donors,
        values,
    }))
}

fn padded(region: SourceRegion, active: SourceRegion, amount: u32) -> SourceRegion {
    let x = region.x.saturating_sub(amount).max(active.x);
    let y = region.y.saturating_sub(amount).max(active.y);
    let right = region
        .x
        .saturating_add(region.width)
        .saturating_add(amount)
        .min(active.x + active.width);
    let bottom = region
        .y
        .saturating_add(region.height)
        .saturating_add(amount)
        .min(active.y + active.height);
    SourceRegion {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

/// Boundary-ranked donor healing for the current repair contract in signed/HDR AP1.
///
/// The returned image preserves every input bit outside `mask`. Masked input
/// values are erased before matching or reconstruction and cannot affect output.
pub fn heal_ranked(
    image: &[Ap1],
    size: [u32; 2],
    mask: &[bool],
    donor: DonorChoice,
) -> Result<(Vec<Ap1>, Vec<DonorMatch>), Error> {
    heal_ranked_with_search(image, size, mask, donor, DonorSearch::Bounded)
}

#[derive(Clone, Copy)]
enum DonorSearch {
    Bounded,
    #[cfg(test)]
    Exhaustive,
}

impl DonorSearch {
    fn is_bounded(self) -> bool {
        matches!(self, Self::Bounded)
    }

    fn ceiling(self, best: f64) -> Option<f64> {
        match self {
            Self::Bounded => Some(competitive_score(best)),
            #[cfg(test)]
            Self::Exhaustive => None,
        }
    }
}

fn competitive_score(best: f64) -> f64 {
    best * 1.01 + 1e-12
}

fn heal_ranked_with_search(
    image: &[Ap1],
    size: [u32; 2],
    mask: &[bool],
    donor: DonorChoice,
    search: DonorSearch,
) -> Result<(Vec<Ap1>, Vec<DonorMatch>), Error> {
    heal_with_execution(image, size, mask, donor, search, &mut Execution::default())
        .map(|result| result.expect("uncancelled donor healing"))
}

type Healed = (Vec<Ap1>, Vec<DonorMatch>);

fn heal_with_execution(
    image: &[Ap1],
    size: [u32; 2],
    mask: &[bool],
    donor: DonorChoice,
    search: DonorSearch,
    execution: &mut Execution<'_>,
) -> Result<Option<Healed>, Error> {
    if (execution.cancelled)() {
        return Ok(None);
    }
    let [width, height] = dimensions(size, image.len())?;
    if mask.len() != image.len() || mask.iter().all(|bit| *bit) {
        return Err(Error::InvalidParameter("invalid repair mask"));
    }
    if image.iter().flatten().any(|value| !value.is_finite()) {
        return Err(Error::InvalidParameter("repair requires finite AP1"));
    }
    if !mask.iter().any(|bit| *bit) {
        return Ok(Some((image.to_vec(), Vec::new())));
    }
    let mut source = image.to_vec();
    for (pixel, hidden) in source.iter_mut().zip(mask) {
        if *hidden {
            *pixel = [0.; 3];
        }
    }
    let groups = grouped_components(mask, width, height);
    let smooth = gaussian_blur(&source, width, height);
    let gradients = sobel(&smooth, width, height);
    let gradient_known = dilate(mask, width, height, 4)
        .into_iter()
        .map(|unknown| !unknown)
        .collect::<Vec<_>>();
    let masked_bounds = selected_bounds(mask, width);
    let mut output = image.to_vec();
    let mut matches = Vec::new();
    for component in groups {
        if (execution.cancelled)() {
            return Ok(None);
        }
        let Some(donor_match) = heal_component(
            &source,
            mask,
            masked_bounds,
            &gradients,
            &gradient_known,
            width,
            height,
            &component,
            donor,
            search,
            &mut output,
            execution,
        )?
        else {
            return Ok(None);
        };
        matches.push(donor_match);
    }
    if output.iter().flatten().any(|value| !value.is_finite())
        || output.iter().zip(image).zip(mask).any(|((a, b), hidden)| {
            !hidden && a.iter().zip(b).any(|(x, y)| x.to_bits() != y.to_bits())
        })
    {
        return Err(Error::NumericalOverflow);
    }
    if (execution.cancelled)() {
        return Ok(None);
    }
    Ok(Some((output, matches)))
}

#[allow(clippy::too_many_arguments)]
fn heal_component(
    image: &[Ap1],
    mask: &[bool],
    masked_bounds: Option<[usize; 4]>,
    gradients: &[[f32; 6]],
    gradient_known: &[bool],
    width: usize,
    height: usize,
    component: &[usize],
    donor_choice: DonorChoice,
    search: DonorSearch,
    output: &mut [Ap1],
    execution: &mut Execution<'_>,
) -> Result<Option<DonorMatch>, Error> {
    let min_x = component.iter().map(|index| index % width).min().unwrap();
    let max_x = component.iter().map(|index| index % width).max().unwrap();
    let min_y = component.iter().map(|index| index / width).min().unwrap();
    let max_y = component.iter().map(|index| index / width).max().unwrap();
    if min_x < 2 || min_y < 2 || max_x + 2 >= width || max_y + 2 >= height {
        return Err(Error::InvalidParameter(
            "biharmonic correction requires a two-pixel exterior",
        ));
    }
    let padding = CONTEXT_PADDING as usize;
    let x0 = min_x.saturating_sub(padding);
    let y0 = min_y.saturating_sub(padding);
    let x1 = (max_x + padding + 1).min(width);
    let y1 = (max_y + padding + 1).min(height);
    let local_width = x1 - x0;
    let local_height = y1 - y0;
    let mut local = vec![false; local_width * local_height];
    for index in component {
        local[(index / width - y0) * local_width + index % width - x0] = true;
    }
    let required = dilate(&local, local_width, local_height, 2);
    let eligibility = DonorEligibility::new(mask, width, masked_bounds, local_width, &required);
    for y in 0..local_height {
        for x in 0..local_width {
            if required[y * local_width + x]
                && mask[(y0 + y) * width + x0 + x]
                && !local[y * local_width + x]
            {
                return Err(Error::InvalidParameter(
                    "dependent repair masks were not grouped",
                ));
            }
        }
    }
    let known = (0..local.len())
        .map(|index| !mask[(y0 + index / local_width) * width + x0 + index % local_width])
        .collect::<Vec<_>>();
    let known_count = known.iter().filter(|bit| **bit).count();
    if known_count == 0 {
        return Err(Error::InvalidParameter("no donor matching context"));
    }
    let mut center = [0.; 3];
    for index in 0..local.len() {
        if known[index] {
            let pixel = image[(y0 + index / local_width) * width + x0 + index % local_width];
            for channel in 0..3 {
                center[channel] += f64::from(pixel[channel]);
            }
        }
    }
    for value in &mut center {
        *value /= known_count as f64;
    }
    let mut variance = 0.;
    for index in 0..local.len() {
        if known[index] {
            let pixel = image[(y0 + index / local_width) * width + x0 + index % local_width];
            for channel in 0..3 {
                variance += (f64::from(pixel[channel]) - center[channel]).powi(2);
            }
        }
    }
    let center_max = center.iter().map(|value| value.abs()).fold(0., f64::max);
    let contrast = (variance / (known_count * 3) as f64)
        .sqrt()
        .max(center_max * 1e-6)
        .max(1e-12);
    let Some(distances) =
        distance_to_unknown(&known, local_width, local_height, execution.cancelled)
    else {
        return Ok(None);
    };
    let mut weights = vec![0.; local.len()];
    for index in 0..local.len() {
        if known[index] {
            weights[index] = 0.15 + 0.85 * (-(distances[index] - 1.).max(0.) / 3.).exp();
        }
    }
    let valid_target = (0..local.len())
        .map(|index| gradient_known[(y0 + index / local_width) * width + x0 + index % local_width])
        .collect::<Vec<_>>();
    if !valid_target.iter().any(|value| *value) {
        return Err(Error::InvalidParameter("no known structure context"));
    }
    let mut gradient_square = 0.;
    let mut gradient_values = 0;
    for index in 0..local.len() {
        if valid_target[index] {
            let value = gradients[(y0 + index / local_width) * width + x0 + index % local_width];
            for channel in value {
                gradient_square += f64::from(channel).powi(2);
                gradient_values += 1;
            }
        }
    }
    let gradient_scale = (gradient_square / gradient_values as f64)
        .sqrt()
        .max(contrast * 1e-6);
    let search_x0 = x0.saturating_sub(128);
    let search_y0 = y0.saturating_sub(128);
    let search_x1 = (x1 + 128).min(width);
    let search_y1 = (y1 + 128).min(height);
    let candidates = match donor_choice {
        DonorChoice::Automatic => {
            let totals = MatchWeights::new(&weights, &valid_target);
            let filter = screening::prepare(
                MatchContext {
                    image,
                    mask,
                    gradients,
                    gradient_known,
                    width,
                    target: [x0, y0],
                    source: [0, 0],
                    size: [local_width, local_height],
                    known: &known,
                    valid_target: &valid_target,
                    weights: &weights,
                    center,
                    contrast,
                    gradient_scale,
                },
                totals,
                &eligibility,
                [
                    search_x0,
                    search_y0,
                    search_x1 - local_width,
                    search_y1 - local_height,
                ],
                search,
                execution,
            )?;
            if (execution.cancelled)() {
                return Ok(None);
            }
            // A seed minimum must retain its actual candidates, especially exact zero:
            // otherwise a cached positive score could incorrectly become the zero winner.
            let mut candidates = filter
                .as_ref()
                .map(|f| {
                    f.scores
                        .iter()
                        .filter(|(_, score)| **score <= competitive_score(f.best))
                        .map(|(p, s)| (*s, p[1], p[0]))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut best = filter.as_ref().map_or(f64::INFINITY, |f| f.best);
            let mut zero_winner = if best == 0. {
                select_donor(&candidates, [x0, y0])
            } else {
                None
            };
            for sy in search_y0..=search_y1 - local_height {
                if (execution.cancelled)() {
                    return Ok(None);
                }
                for sx in search_x0..=search_x1 - local_width {
                    if (execution.cancelled)() {
                        return Ok(None);
                    }
                    if filter
                        .as_ref()
                        .is_some_and(|f| f.scores.contains_key(&[sx, sy]))
                    {
                        continue;
                    }
                    if zero_winner
                        .is_some_and(|winner| !can_beat_zero_winner(winner, [sx, sy], [x0, y0]))
                    {
                        continue;
                    }
                    if !filter.as_ref().map_or_else(
                        || eligibility.clean(sx, sy, search),
                        |f| f.allows(sx, sy, best),
                    ) {
                        continue;
                    }
                    let context = MatchContext {
                        image,
                        mask,
                        gradients,
                        gradient_known,
                        width,
                        target: [x0, y0],
                        source: [sx, sy],
                        size: [local_width, local_height],
                        known: &known,
                        valid_target: &valid_target,
                        weights: &weights,
                        center,
                        contrast,
                        gradient_scale,
                    };
                    let score = matching_score_bounded(context, totals, search.ceiling(best));
                    if let Ok(Some(score)) = score {
                        best = best.min(score);
                        let candidate = (score, sy, sx);
                        candidates.push(candidate);
                        if search.is_bounded() && best == 0. {
                            zero_winner = match zero_winner {
                                Some(winner) => Some(
                                    if score <= competitive_score(0.)
                                        && donor_order(&candidate, &winner, [x0, y0]).is_lt()
                                    {
                                        candidate
                                    } else {
                                        winner
                                    },
                                ),
                                None => select_donor(&candidates, [x0, y0]),
                            };
                        }
                    }
                }
            }
            if candidates.is_empty() {
                return Err(Error::InvalidParameter(
                    "no eligible donor in search region",
                ));
            }
            candidates
        }
        DonorChoice::Offset([dx, dy]) => {
            let sx = offset(x0, dx).ok_or(Error::InvalidParameter("manual donor outside crop"))?;
            let sy = offset(y0, dy).ok_or(Error::InvalidParameter("manual donor outside crop"))?;
            if sx + local_width > width
                || sy + local_height > height
                || !eligibility.clean(sx, sy, search)
            {
                return Err(Error::InvalidParameter(
                    "manual donor intersects required pixels",
                ));
            }
            vec![(
                matching_score(MatchContext {
                    image,
                    mask,
                    gradients,
                    gradient_known,
                    width,
                    target: [x0, y0],
                    source: [sx, sy],
                    size: [local_width, local_height],
                    known: &known,
                    valid_target: &valid_target,
                    weights: &weights,
                    center,
                    contrast,
                    gradient_scale,
                })?,
                sy,
                sx,
            )]
        }
    };
    let (matching_score, source_y, source_x) = select_donor(&candidates, [x0, y0]).unwrap();
    let mut difference = vec![[0.; 3]; local.len()];
    let mut paired_context_pixels = 0;
    let mut context_error = 0.;
    for y in 0..local_height {
        for x in 0..local_width {
            let index = y * local_width + x;
            let target = image[(y0 + y) * width + x0 + x];
            let source_pixel = image[(source_y + y) * width + source_x + x];
            if known[index] && !mask[(source_y + y) * width + source_x + x] {
                paired_context_pixels += 1;
                for channel in 0..3 {
                    context_error +=
                        (f64::from(target[channel]) - f64::from(source_pixel[channel])).powi(2);
                }
            }
            if required[index] && !local[index] {
                difference[index] =
                    std::array::from_fn(|channel| target[channel] - source_pixel[channel]);
            }
        }
    }
    if (execution.cancelled)() {
        return Ok(None);
    }
    let correction = limited_biharmonic(&difference, &local, local_width, local_height)?;
    if (execution.cancelled)() {
        return Ok(None);
    }
    for y in 0..local_height {
        for x in 0..local_width {
            let index = y * local_width + x;
            if local[index] {
                let donor_pixel = image[(source_y + y) * width + source_x + x];
                output[(y0 + y) * width + x0 + x] = std::array::from_fn(|channel| {
                    donor_pixel[channel] + correction[index][channel]
                });
            }
        }
    }
    Ok(Some(DonorMatch {
        target: [
            x0 as u32,
            y0 as u32,
            local_width as u32,
            local_height as u32,
        ],
        source: [source_x as u32, source_y as u32],
        context_mse: context_error / (paired_context_pixels * 3) as f64,
        paired_context_pixels,
        matching_score,
    }))
}

fn donor_order(
    a: &(f64, usize, usize),
    b: &(f64, usize, usize),
    target: [usize; 2],
) -> std::cmp::Ordering {
    let da = distance(a.2, a.1, target[0], target[1]);
    let db = distance(b.2, b.1, target[0], target[1]);
    da.total_cmp(&db)
        .then_with(|| a.0.total_cmp(&b.0))
        .then_with(|| a.1.cmp(&b.1))
        .then_with(|| a.2.cmp(&b.2))
}

fn select_donor(
    candidates: &[(f64, usize, usize)],
    target: [usize; 2],
) -> Option<(f64, usize, usize)> {
    let best = candidates.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    candidates
        .iter()
        .filter(|c| c.0 <= competitive_score(best))
        .min_by(|a, b| donor_order(a, b, target))
        .copied()
}

fn can_beat_zero_winner(
    winner: (f64, usize, usize),
    source: [usize; 2],
    target: [usize; 2],
) -> bool {
    // Zero fixes the global best and epsilon margin. Even a perfect new score must beat
    // the incumbent's complete distance/score/y/x rank; nearer nonzero epsilon ties survive.
    donor_order(&(0., source[1], source[0]), &winner, target).is_lt()
}

#[derive(Clone, Copy)]
struct MatchContext<'a> {
    image: &'a [Ap1],
    mask: &'a [bool],
    gradients: &'a [[f32; 6]],
    gradient_known: &'a [bool],
    width: usize,
    target: [usize; 2],
    source: [usize; 2],
    size: [usize; 2],
    known: &'a [bool],
    valid_target: &'a [bool],
    weights: &'a [f64],
    center: [f64; 3],
    contrast: f64,
    gradient_scale: f64,
}

#[derive(Clone, Copy)]
struct MatchWeights {
    color: f64,
    gradient: f64,
}

impl MatchWeights {
    fn new(weights: &[f64], valid_target: &[bool]) -> Self {
        Self {
            color: weights.iter().sum(),
            gradient: weights
                .iter()
                .zip(valid_target)
                .filter_map(|(weight, valid)| valid.then_some(*weight))
                .sum(),
        }
    }
}

fn matching_score(context: MatchContext<'_>) -> Result<f64, Error> {
    matching_score_bounded(
        context,
        MatchWeights::new(context.weights, context.valid_target),
        None,
    )
    .map(|score| score.expect("unbounded matching never prunes"))
}

fn matching_score_bounded(
    context: MatchContext<'_>,
    totals: MatchWeights,
    ceiling: Option<f64>,
) -> Result<Option<f64>, Error> {
    let [local_width, local_height] = context.size;
    let mut color_weight = 0.;
    let mut color_error = 0.;
    let mut gradient_weight = 0.;
    let mut gradient_error = 0.;
    for y in 0..local_height {
        for x in 0..local_width {
            let local = y * local_width + x;
            let target = (context.target[1] + y) * context.width + context.target[0] + x;
            let source = (context.source[1] + y) * context.width + context.source[0] + x;
            let weight = context.weights[local];
            if context.known[local] && !context.mask[source] {
                color_weight += weight;
                for channel in 0..3 {
                    let a = (f64::from(context.image[target][channel]) - context.center[channel])
                        / context.contrast;
                    let b = (f64::from(context.image[source][channel]) - context.center[channel])
                        / context.contrast;
                    color_error += (a - b).powi(2) * weight;
                }
            }
            if context.valid_target[local] && context.gradient_known[source] {
                gradient_weight += weight;
                for channel in 0..6 {
                    let delta = (f64::from(context.gradients[target][channel])
                        - f64::from(context.gradients[source][channel]))
                        / context.gradient_scale;
                    gradient_error += delta.powi(2) * weight;
                }
            }
        }
        // Errors only increase and paired weights cannot exceed these target totals.
        // A strict lower bound outside the competitive margin can never become a winner.
        if ceiling.is_some_and(|limit| {
            0.75 * color_error / (totals.color * 3.)
                + 0.25 * gradient_error / (totals.gradient * 6.)
                > limit
        }) {
            return Ok(None);
        }
    }
    let required_color = totals.color * 0.5;
    let required_gradient = totals.gradient * 0.5;
    if color_weight < required_color.max(1e-12) || gradient_weight < required_gradient.max(1e-12) {
        return Err(Error::InvalidParameter("insufficient paired donor context"));
    }
    Ok(Some(
        0.75 * color_error / (color_weight * 3.) + 0.25 * gradient_error / (gradient_weight * 6.),
    ))
}

fn dimensions(size: [u32; 2], count: usize) -> Result<[usize; 2], Error> {
    let width = usize::try_from(size[0]).unwrap();
    let height = usize::try_from(size[1]).unwrap();
    if width.checked_mul(height) != Some(count) || count == 0 {
        return Err(Error::InvalidParameter("image dimensions disagree"));
    }
    Ok([width, height])
}

fn offset(value: usize, delta: i32) -> Option<usize> {
    if delta < 0 {
        value.checked_sub(delta.unsigned_abs() as usize)
    } else {
        value.checked_add(delta as usize)
    }
}

fn distance(x: usize, y: usize, tx: usize, ty: usize) -> f64 {
    ((x as f64 - tx as f64).powi(2) + (y as f64 - ty as f64).powi(2)).sqrt()
}

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

struct DonorEligibility<'a> {
    mask: &'a [bool],
    width: usize,
    masked: Option<[usize; 4]>,
    needed: Option<[usize; 4]>,
    offsets: Vec<usize>,
    #[cfg(test)]
    required: &'a [bool],
    #[cfg(test)]
    local_width: usize,
}

impl<'a> DonorEligibility<'a> {
    fn new(
        mask: &'a [bool],
        width: usize,
        masked: Option<[usize; 4]>,
        local_width: usize,
        required: &'a [bool],
    ) -> Self {
        Self {
            mask,
            width,
            masked,
            needed: selected_bounds(required, local_width),
            offsets: required
                .iter()
                .enumerate()
                .filter_map(|(i, needed)| {
                    needed.then_some(i / local_width * width + i % local_width)
                })
                .collect(),
            #[cfg(test)]
            required,
            #[cfg(test)]
            local_width,
        }
    }

    fn clean(&self, x: usize, y: usize, _search: DonorSearch) -> bool {
        #[cfg(test)]
        if matches!(_search, DonorSearch::Exhaustive) {
            return self.required.iter().enumerate().all(|(i, needed)| {
                !needed
                    || !self.mask
                        [(y + i / self.local_width) * self.width + x + i % self.local_width]
            });
        }
        let (Some(masked), Some(needed)) = (self.masked, self.needed) else {
            return true;
        };
        // Disjoint bounding boxes prove cleanliness; intersecting boxes still test exact support.
        if x + needed[2] <= masked[0]
            || y + needed[3] <= masked[1]
            || x + needed[0] >= masked[2]
            || y + needed[1] >= masked[3]
        {
            return true;
        }
        let base = y * self.width + x;
        self.offsets.iter().all(|offset| !self.mask[base + offset])
    }
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

fn exact_distance_squares(width: usize, height: usize) -> bool {
    (width.saturating_sub(1) as u64)
        .checked_pow(2)
        .and_then(|x| {
            (height.saturating_sub(1) as u64)
                .checked_pow(2)
                .and_then(|y| x.checked_add(y))
        })
        .is_some_and(|n| n <= 1u64 << 53)
}

fn distance_to_unknown(
    known: &[bool],
    width: usize,
    height: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<f64>> {
    // Integer squared distances are identical to the reference's binary64 sums
    // in this range. Apply the same sqrt once to the exact minimum, not once
    // per possible unknown pixel. Keep the scalar path beyond that guarantee.
    let exact = exact_distance_squares(width, height);

    #[derive(Clone, Copy)]
    struct Site {
        base: u64,
        position: u32,
        start: u32,
    }
    if !exact || std::mem::size_of::<Site>() > std::mem::size_of::<(usize, usize)>() {
        return distance_to_unknown_scalar(known, width, cancelled);
    }
    if known.is_empty() {
        return Some(Vec::new());
    }
    let mut values = vec![f64::INFINITY; known.len()];
    for y in 0..height {
        let mut previous = None;
        for x in 0..width {
            let i = y * width + x;
            if i.is_multiple_of(128) && cancelled() {
                return None;
            }
            if !known[i] {
                previous = Some(x);
            }
            if let Some(p) = previous {
                values[i] = (x.abs_diff(p) as u64).pow(2) as f64;
            }
        }
        previous = None;
        for x in (0..width).rev() {
            let i = y * width + x;
            if i.is_multiple_of(128) && cancelled() {
                return None;
            }
            if !known[i] {
                previous = Some(x);
            }
            if let Some(p) = previous {
                values[i] = values[i].min((x.abs_diff(p) as u64).pow(2) as f64);
            }
        }
    }
    // At most one site per row containing an unknown; each site uses no more
    // storage than a scalar-reference unknown coordinate on supported targets.
    let mut envelope: Vec<Site> = Vec::new();
    for x in 0..width {
        envelope.clear();
        for q in 0..height {
            if q.is_multiple_of(128) && cancelled() {
                return None;
            }
            let base = values[q * width + x];
            if !base.is_finite() {
                continue;
            }
            let base = base as u64;
            let start = loop {
                let Some(p) = envelope.last().copied() else {
                    break 0;
                };
                let numerator = (q as i64).pow(2) + base as i64
                    - (i64::from(p.position).pow(2) + p.base as i64);
                // First integer row where the new parabola is strictly lower.
                // Clipping only its active domain preserves equal-distance ties.
                let first = (numerator.div_euclid(2 * (q as i64 - i64::from(p.position))) + 1)
                    .clamp(0, height as i64) as u32;
                if first <= p.start {
                    envelope.pop();
                } else {
                    break first;
                }
            };
            if start < height as u32 {
                envelope.push(Site {
                    base,
                    position: q as u32,
                    start,
                });
            }
        }
        if envelope.is_empty() {
            continue;
        }
        let mut active = 0;
        for y in 0..height {
            if y.is_multiple_of(128) && cancelled() {
                return None;
            }
            while active + 1 < envelope.len() && envelope[active + 1].start <= y as u32 {
                active += 1;
            }
            let p = envelope[active];
            let squared = p.base + (y.abs_diff(p.position as usize) as u64).pow(2);
            values[y * width + x] = (squared as f64).sqrt();
        }
    }
    Some(values)
}

fn distance_to_unknown_scalar(
    known: &[bool],
    width: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<f64>> {
    let unknown = known
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (!value).then_some((index % width, index / width)))
        .collect::<Vec<_>>();
    known
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index % 128 == 0 && cancelled() {
                return None;
            }
            Some(if !value {
                0.
            } else {
                let x = index % width;
                let y = index / width;
                unknown
                    .iter()
                    .map(|(ux, uy)| distance(x, y, *ux, *uy))
                    .fold(f64::INFINITY, f64::min)
            })
        })
        .collect()
}

fn gaussian_blur(image: &[Ap1], width: usize, height: usize) -> Vec<Ap1> {
    let mut kernel = [0.; 7];
    for (index, value) in kernel.iter_mut().enumerate() {
        *value = (-0.5 * (index as f64 - 3.).powi(2)).exp();
    }
    let sum = kernel.iter().sum::<f64>();
    for value in &mut kernel {
        *value /= sum;
    }
    let mut horizontal = vec![[0.; 3]; image.len()];
    for y in 0..height {
        for x in 0..width {
            for (tap, weight) in kernel.iter().enumerate() {
                let sx = reflected(x as isize + tap as isize - 3, width);
                for channel in 0..3 {
                    horizontal[y * width + x][channel] +=
                        image[y * width + sx][channel] * *weight as f32;
                }
            }
        }
    }
    let mut output = vec![[0.; 3]; image.len()];
    for y in 0..height {
        for x in 0..width {
            for (tap, weight) in kernel.iter().enumerate() {
                let sy = reflected(y as isize + tap as isize - 3, height);
                for channel in 0..3 {
                    output[y * width + x][channel] +=
                        horizontal[sy * width + x][channel] * *weight as f32;
                }
            }
        }
    }
    output
}

fn sobel(image: &[Ap1], width: usize, height: usize) -> Vec<[f32; 6]> {
    let mut output = vec![[0.; 6]; image.len()];
    let smooth = [1., 2., 1.];
    let derivative = [-1., 0., 1.];
    for y in 0..height {
        for x in 0..width {
            for ky in 0..3 {
                for kx in 0..3 {
                    let sy = reflected(y as isize + ky as isize - 1, height);
                    let sx = reflected(x as isize + kx as isize - 1, width);
                    for channel in 0..3 {
                        output[y * width + x][channel] +=
                            image[sy * width + sx][channel] * derivative[kx] * smooth[ky] / 8.;
                        output[y * width + x][channel + 3] +=
                            image[sy * width + sx][channel] * smooth[kx] * derivative[ky] / 8.;
                    }
                }
            }
        }
    }
    output
}

fn reflected(mut value: isize, size: usize) -> usize {
    if size == 1 {
        return 0;
    }
    let size = size as isize;
    while value < 0 || value >= size {
        value = if value < 0 {
            -value
        } else {
            2 * size - value - 2
        };
    }
    value as usize
}

fn harmonic_fill(
    image: &[Ap1],
    mask: &[bool],
    width: usize,
    height: usize,
) -> Result<Vec<Ap1>, Error> {
    if !mask.iter().any(|value| *value) {
        return Ok(image.to_vec());
    }
    if mask.iter().all(|value| *value) {
        return Err(Error::InvalidParameter("repair has no valid boundary"));
    }
    let unknown = mask
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.then_some(index))
        .collect::<Vec<_>>();
    let mut positions = vec![usize::MAX; mask.len()];
    for (position, index) in unknown.iter().enumerate() {
        positions[*index] = position;
    }
    let mut degree = vec![0.; unknown.len()];
    let mut neighbors = vec![Vec::new(); unknown.len()];
    let mut rhs = vec![[0.; 3]; unknown.len()];
    for (position, index) in unknown.iter().enumerate() {
        let x = index % width;
        let y = index / width;
        for (nx, ny) in neighbors4(x, y, width, height) {
            degree[position] += 1.;
            let next = ny * width + nx;
            if mask[next] {
                neighbors[position].push(positions[next]);
            } else {
                for channel in 0..3 {
                    rhs[position][channel] += f64::from(image[next][channel]);
                }
            }
        }
    }
    let apply = |input: &[f64], output: &mut [f64]| {
        for index in 0..unknown.len() {
            output[index] = degree[index] * input[index]
                - neighbors[index]
                    .iter()
                    .map(|next| input[*next])
                    .sum::<f64>();
        }
    };
    let mut solved = vec![[0.; 3]; unknown.len()];
    for channel in 0..3 {
        let b = rhs.iter().map(|value| value[channel]).collect::<Vec<_>>();
        let x = conjugate_gradient(&b, &apply)?;
        for (value, solved) in x.into_iter().zip(&mut solved) {
            solved[channel] = value;
        }
    }
    let mut output = image.to_vec();
    for (index, solved) in unknown.into_iter().zip(solved) {
        output[index] = solved.map(|value| value as f32);
    }
    Ok(output)
}

fn conjugate_gradient(rhs: &[f64], apply: &impl Fn(&[f64], &mut [f64])) -> Result<Vec<f64>, Error> {
    let mut x = vec![0.; rhs.len()];
    let mut residual = rhs.to_vec();
    let mut direction = residual.clone();
    let mut squared = dot(&residual, &residual);
    let tolerance = squared.sqrt().max(1.) * 1e-12;
    if squared.sqrt() <= tolerance {
        return Ok(x);
    }
    let mut product = vec![0.; rhs.len()];
    for _ in 0..(rhs.len() * 10 + 100) {
        apply(&direction, &mut product);
        let denominator = dot(&direction, &product);
        if !denominator.is_finite() || denominator <= 0. {
            return Err(Error::NumericalOverflow);
        }
        let alpha = squared / denominator;
        for index in 0..rhs.len() {
            x[index] += alpha * direction[index];
            residual[index] -= alpha * product[index];
        }
        let next_squared = dot(&residual, &residual);
        if next_squared.sqrt() <= tolerance {
            return Ok(x);
        }
        let beta = next_squared / squared;
        for index in 0..rhs.len() {
            direction[index] = residual[index] + beta * direction[index];
        }
        squared = next_squared;
    }
    Err(Error::NumericalOverflow)
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn neighbors4(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> impl Iterator<Item = (usize, usize)> {
    [
        x.checked_sub(1).map(|x| (x, y)),
        (x + 1 < width).then_some((x + 1, y)),
        y.checked_sub(1).map(|y| (x, y)),
        (y + 1 < height).then_some((x, y + 1)),
    ]
    .into_iter()
    .flatten()
}

fn limited_biharmonic(
    image: &[Ap1],
    mask: &[bool],
    width: usize,
    height: usize,
) -> Result<Vec<Ap1>, Error> {
    let mut correction = harmonic_fill(image, mask, width, height)?
        .into_iter()
        .map(|pixel| pixel.map(f64::from))
        .collect::<Vec<_>>();
    const KERNEL: [(isize, isize, f64); 13] = [
        (0, 0, 20.),
        (-1, 0, -8.),
        (1, 0, -8.),
        (0, -1, -8.),
        (0, 1, -8.),
        (-2, 0, 1.),
        (2, 0, 1.),
        (0, -2, 1.),
        (0, 2, 1.),
        (-1, -1, 2.),
        (-1, 1, 2.),
        (1, -1, 2.),
        (1, 1, 2.),
    ];
    for _ in 0..8 {
        let previous = correction.clone();
        for index in 0..mask.len() {
            if !mask[index] {
                continue;
            }
            let x = index % width;
            let y = index / width;
            for channel in 0..3 {
                let residual = KERNEL
                    .iter()
                    .map(|(dx, dy, weight)| {
                        let nx = (x as isize + dx) as usize;
                        let ny = (y as isize + dy) as usize;
                        weight * previous[ny * width + nx][channel]
                    })
                    .sum::<f64>();
                correction[index][channel] -= residual / 64.;
            }
        }
    }
    let mut output = image.to_vec();
    for index in 0..mask.len() {
        if mask[index] {
            output[index] = correction[index].map(|value| value as f32);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
