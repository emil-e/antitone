//! Source-owned dust-repair instructions and versioned AP1 donor healer.
//!
//! Repair support is binary and source-coordinate owned. Manual and detected
//! supports differ only in UI provenance; both enter [`heal_ranked`] identically.
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

mod matching;
mod support;
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
    validate_offset(repair.offset, &repair.support, active)?;
    let near = NearSupport::new(&repair.support);
    let groups = repair.support.grouped_supports();
    let mut values = vec![[0.; 3]; (repair.support.size[0] * repair.support.size[1]) as usize];
    let mut donors = Vec::new();
    for group in &groups {
        if (execution.cancelled)() {
            return Ok(None);
        }
        let Some(target) =
            build_search_target(&repair.support, group, active, &mut base, execution)?
        else {
            continue;
        };
        let Some(donor) = fetch_crop(
            &mut base,
            target.origin,
            target.size,
            repair.offset,
            execution,
        )?
        else {
            return Ok(None);
        };
        let Some((donor_match, component)) = heal_crop(
            &repair.support,
            &near,
            &target,
            &donor,
            repair.offset,
            execution,
        )?
        else {
            return Ok(None);
        };
        for (index, value) in component {
            values[index] = value;
        }
        donors.push(donor_match);
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

fn fetch_crop(
    base: &mut impl FnMut([u32; 2]) -> Result<Ap1, Error>,
    origin: [u32; 2],
    size: [usize; 2],
    offset: [i32; 2],
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<Ap1>>, Error> {
    let x0 = origin[0] as i32 + offset[0];
    let y0 = origin[1] as i32 + offset[1];
    let mut crop = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        for x in 0..size[0] {
            if (execution.cancelled)() {
                return Ok(None);
            }
            crop.push(base([(x0 + x as i32) as u32, (y0 + y as i32) as u32])?);
        }
    }
    Ok(Some(crop))
}

type HealedComponent = (DonorMatch, Vec<(usize, Ap1)>);

#[allow(clippy::needless_range_loop)]
fn heal_crop(
    support: &SupportMask,
    near: &NearSupport,
    target: &SearchTarget,
    donor: &[Ap1],
    offset: [i32; 2],
    execution: &mut Execution<'_>,
) -> Result<Option<HealedComponent>, Error> {
    let [width, height] = target.size;
    if !target.has_exterior {
        return Err(Error::InvalidParameter(
            "biharmonic correction requires a two-pixel exterior",
        ));
    }
    let mut difference = vec![[0.; 3]; target.local.len()];
    for index in 0..target.local.len() {
        if target.required[index] && !target.local[index] {
            difference[index] =
                std::array::from_fn(|channel| target.image[index][channel] - donor[index][channel]);
        }
    }
    let correction = limited_biharmonic(&difference, &target.local, width, height)?;
    if (execution.cancelled)() {
        return Ok(None);
    }
    let mut component = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if target.local[index] {
                let point = [target.origin[0] + x as u32, target.origin[1] + y as u32];
                let support_index = ((point[1] - support.origin[1]) * support.size[0] + point[0]
                    - support.origin[0]) as usize;
                component.push((
                    support_index,
                    std::array::from_fn(|channel| {
                        donor[index][channel] + correction[index][channel]
                    }),
                ));
            }
        }
    }
    let Some(smooth) = gaussian_blur(donor, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let Some(donor_gradients) = sobel(&smooth, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let (color_error, color_weight, gradient_error, gradient_weight) =
        crop_errors(target, support, near, offset, donor, &donor_gradients);
    let color = if color_weight > 0. {
        0.75 * color_error / (color_weight * 3.)
    } else {
        0.
    };
    let gradient = if gradient_weight > 0. {
        0.25 * gradient_error / (gradient_weight * 6.)
    } else {
        0.
    };
    let matching_score = if color_weight > 0. || gradient_weight > 0. {
        color + gradient
    } else {
        f64::INFINITY
    };
    let mut paired_context_pixels = 0usize;
    let mut context_error = 0.;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let donor_point = [
                target.origin[0] as i32 + x as i32 + offset[0],
                target.origin[1] as i32 + y as i32 + offset[1],
            ];
            let donor_is_support = donor_point[0] >= 0
                && donor_point[1] >= 0
                && support.contains([donor_point[0] as u32, donor_point[1] as u32]);
            if target.known[index] && !donor_is_support {
                paired_context_pixels += 1;
                for channel in 0..3 {
                    context_error += (f64::from(target.image[index][channel])
                        - f64::from(donor[index][channel]))
                    .powi(2);
                }
            }
        }
    }
    Ok(Some((
        DonorMatch {
            target: [
                target.origin[0],
                target.origin[1],
                width as u32,
                height as u32,
            ],
            source: [
                (target.origin[0] as i32 + offset[0]) as u32,
                (target.origin[1] as i32 + offset[1]) as u32,
            ],
            context_mse: if paired_context_pixels > 0 {
                context_error / (paired_context_pixels * 3) as f64
            } else {
                0.
            },
            paired_context_pixels,
            matching_score,
        },
        component,
    )))
}

pub fn heal_ranked(
    image: &[Ap1],
    size: [u32; 2],
    mask: &[bool],
    offset: [i32; 2],
) -> Result<(Vec<Ap1>, Vec<DonorMatch>), Error> {
    if !mask.iter().any(|hidden| *hidden) {
        return Ok((image.to_vec(), Vec::new()));
    }
    let support = SupportMask::new([0, 0], size, mask.to_vec())?;
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support,
        offset,
    };
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: size[0],
        height: size[1],
    };
    let Some(evaluated) = evaluate_with(
        &repair,
        active,
        |[x, y]| Ok(image[(y as usize) * (size[0] as usize) + x as usize]),
        &mut Execution::default(),
    )?
    else {
        unreachable!("uncancelled donor healing");
    };
    let mut output = image.to_vec();
    for (point, value) in evaluated.pixels() {
        output[(point[1] as usize) * (size[0] as usize) + point[0] as usize] = value;
    }
    Ok((output, evaluated.donors))
}

#[allow(dead_code)]
struct SearchTarget {
    has_exterior: bool,
    origin: [u32; 2],
    size: [usize; 2],
    image: Vec<Ap1>,
    local: Vec<bool>,
    required: Vec<bool>,
    known: Vec<bool>,
    valid_target: Vec<bool>,
    weights: Vec<f64>,
    gradients: Vec<[f32; 6]>,
    center: [f64; 3],
    contrast: f64,
    gradient_scale: f64,
}

/// One target crop for the best-effort donor search. The donor is scored from
/// its own small crop, so candidate distance never changes the fetch size.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
fn build_search_target(
    support: &SupportMask,
    group: &SupportMask,
    active: SourceRegion,
    base: &mut impl FnMut([u32; 2]) -> Result<Ap1, Error>,
    execution: &mut Execution<'_>,
) -> Result<Option<SearchTarget>, Error> {
    let points = group.selected_points().collect::<Vec<_>>();
    if points.is_empty() {
        return Ok(None);
    }
    let min_x = points.iter().map(|p| p[0]).min().unwrap();
    let max_x = points.iter().map(|p| p[0]).max().unwrap();
    let min_y = points.iter().map(|p| p[1]).min().unwrap();
    let max_y = points.iter().map(|p| p[1]).max().unwrap();
    let x0 = min_x.saturating_sub(CONTEXT_PADDING).max(active.x);
    let y0 = min_y.saturating_sub(CONTEXT_PADDING).max(active.y);
    let x1 = (max_x + CONTEXT_PADDING + 1).min(active.x + active.width);
    let y1 = (max_y + CONTEXT_PADDING + 1).min(active.y + active.height);
    let width = (x1 - x0) as usize;
    let height = (y1 - y0) as usize;
    let mut image = vec![[0.; 3]; width * height];
    let mut known = vec![false; width * height];
    let mut local = vec![false; width * height];
    let mut mask = vec![false; width * height];
    for y in 0..height {
        if (execution.cancelled)() {
            return Ok(None);
        }
        for x in 0..width {
            let point = [x0 + x as u32, y0 + y as u32];
            let index = y * width + x;
            let masked = support.contains(point);
            mask[index] = masked;
            local[index] = group.contains(point);
            known[index] = !masked;
            if !masked {
                image[index] = base(point)?;
            }
        }
    }
    let required = dilate(&local, width, height, 2);
    for index in 0..local.len() {
        if required[index] && mask[index] && !local[index] {
            return Err(Error::InvalidParameter(
                "dependent repair masks were not grouped",
            ));
        }
    }
    let known_count = known.iter().filter(|bit| **bit).count();
    if known_count == 0 {
        return Ok(None);
    }
    let Some(smooth) = gaussian_blur(&image, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let Some(gradients) = sobel(&smooth, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let gradient_known = dilate(&mask, width, height, 4)
        .into_iter()
        .map(|unknown| !unknown)
        .collect::<Vec<_>>();
    let valid_target = (0..local.len())
        .map(|index| gradient_known[index])
        .collect::<Vec<_>>();
    let mut center = [0.; 3];
    for index in 0..local.len() {
        if known[index] {
            for channel in 0..3 {
                center[channel] += f64::from(image[index][channel]);
            }
        }
    }
    for value in &mut center {
        *value /= known_count as f64;
    }
    let mut variance = 0.;
    for index in 0..local.len() {
        if known[index] {
            for channel in 0..3 {
                variance += (f64::from(image[index][channel]) - center[channel]).powi(2);
            }
        }
    }
    let center_max = center.iter().map(|value| value.abs()).fold(0., f64::max);
    let contrast = (variance / (known_count * 3) as f64)
        .sqrt()
        .max(center_max * 1e-6)
        .max(1e-12);
    let Some(distances) = distance_to_unknown(&known, width, height, execution.cancelled) else {
        return Ok(None);
    };
    let mut weights = vec![0.; local.len()];
    for index in 0..local.len() {
        if known[index] {
            weights[index] = 0.15 + 0.85 * (-(distances[index] - 1.).max(0.) / 3.).exp();
        }
    }
    let mut gradient_square = 0.;
    let mut gradient_values = 0;
    for index in 0..local.len() {
        if valid_target[index] {
            for channel in gradients[index] {
                gradient_square += f64::from(channel).powi(2);
                gradient_values += 1;
            }
        }
    }
    let gradient_scale = if gradient_values == 0 {
        1.
    } else {
        (gradient_square / gradient_values as f64)
            .sqrt()
            .max(contrast * 1e-6)
    };
    Ok(Some(SearchTarget {
        has_exterior: min_x - x0 >= 2 && min_y - y0 >= 2 && x1 - max_x >= 3 && y1 - max_y >= 3,
        origin: [x0, y0],
        size: [width, height],
        image,
        local,
        required,
        known,
        valid_target,
        weights,
        gradients,
        center,
        contrast,
        gradient_scale,
    }))
}

struct NearSupport {
    origin: [i32; 2],
    size: [usize; 2],
    bits: Vec<bool>,
}

impl NearSupport {
    fn new(support: &SupportMask) -> Self {
        let width = support.size[0] as usize + 8;
        let height = support.size[1] as usize + 8;
        let mut bits = vec![false; width * height];
        for [x, y] in support.selected_points() {
            let local_x = (x - support.origin[0]) as usize + 4;
            let local_y = (y - support.origin[1]) as usize + 4;
            bits[local_y * width + local_x] = true;
        }
        let bits = dilate(&bits, width, height, 4);
        Self {
            origin: [support.origin[0] as i32 - 4, support.origin[1] as i32 - 4],
            size: [width, height],
            bits,
        }
    }

    fn contains(&self, point: [i32; 2]) -> bool {
        let x = point[0] - self.origin[0];
        let y = point[1] - self.origin[1];
        x >= 0
            && y >= 0
            && (x as usize) < self.size[0]
            && (y as usize) < self.size[1]
            && self.bits[y as usize * self.size[0] + x as usize]
    }
}

#[allow(clippy::needless_range_loop)]
fn crop_errors(
    target: &SearchTarget,
    support: &SupportMask,
    near: &NearSupport,
    offset: [i32; 2],
    donor: &[Ap1],
    donor_gradients: &[[f32; 6]],
) -> (f64, f64, f64, f64) {
    let [width, _height] = target.size;
    let mut color_weight = 0.;
    let mut color_error = 0.;
    let mut gradient_weight = 0.;
    let mut gradient_error = 0.;
    for index in 0..target.weights.len() {
        let local_x = index % width;
        let local_y = index / width;
        let donor_point = [
            target.origin[0] as i32 + local_x as i32 + offset[0],
            target.origin[1] as i32 + local_y as i32 + offset[1],
        ];
        let weight = target.weights[index];
        let donor_is_support = donor_point[0] >= 0
            && donor_point[1] >= 0
            && support.contains([donor_point[0] as u32, donor_point[1] as u32]);
        if target.known[index] && !donor_is_support {
            color_weight += weight;
            for channel in 0..3 {
                let a = (f64::from(target.image[index][channel]) - target.center[channel])
                    / target.contrast;
                let b =
                    (f64::from(donor[index][channel]) - target.center[channel]) / target.contrast;
                color_error += (a - b).powi(2) * weight;
            }
        }
        if target.valid_target[index] && !near.contains(donor_point) {
            gradient_weight += weight;
            for channel in 0..6 {
                let delta = (f64::from(target.gradients[index][channel])
                    - f64::from(donor_gradients[index][channel]))
                    / target.gradient_scale;
                gradient_error += delta.powi(2) * weight;
            }
        }
    }
    (color_error, color_weight, gradient_error, gradient_weight)
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

fn gaussian_blur(
    image: &[Ap1],
    width: usize,
    height: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Ap1>> {
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
        if cancelled() {
            return None;
        }
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
        if cancelled() {
            return None;
        }
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
    Some(output)
}

fn sobel(
    image: &[Ap1],
    width: usize,
    height: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<[f32; 6]>> {
    let mut output = vec![[0.; 6]; image.len()];
    let smooth = [1., 2., 1.];
    let derivative = [-1., 0., 1.];
    for y in 0..height {
        if cancelled() {
            return None;
        }
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
    Some(output)
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
mod matching_tests;
#[cfg(test)]
mod tests;
