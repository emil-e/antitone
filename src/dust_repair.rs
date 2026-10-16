//! Provisional, non-persistent dust-repair model and frozen AP1 donor healer.
//!
//! Repair support is binary and source-coordinate owned. Manual and detected
//! supports differ only in UI provenance; both enter [`heal_ranked`] identically.
use crate::{Error, geometry::SourceRegion};
use std::collections::VecDeque;

pub type Ap1 = [f32; 3];
pub(crate) const CONTEXT_PADDING: u32 = 10;
pub(crate) const AUTOMATIC_PADDING: u32 = 128 + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RepairId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairOrigin {
    Manual,
    Detected,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DonorChoice {
    #[default]
    Automatic,
    Offset([i32; 2]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportMask {
    origin: [u32; 2],
    size: [u32; 2],
    bits: Vec<bool>,
}
impl SupportMask {
    pub fn new(origin: [u32; 2], size: [u32; 2], bits: Vec<bool>) -> Result<Self, Error> {
        let count = usize::try_from(
            size[0]
                .checked_mul(size[1])
                .filter(|count| *count > 0)
                .ok_or(Error::InvalidParameter("invalid repair support dimensions"))?,
        )
        .map_err(|_| Error::InvalidParameter("repair support is too large"))?;
        if bits.len() != count || !bits.iter().any(|bit| *bit) {
            return Err(Error::InvalidParameter("invalid repair support bits"));
        }
        origin[0]
            .checked_add(size[0])
            .and_then(|_| origin[1].checked_add(size[1]))
            .ok_or(Error::InvalidParameter(
                "repair support coordinates overflow",
            ))?;
        Ok(Self { origin, size, bits })
    }
    pub fn origin(&self) -> [u32; 2] {
        self.origin
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn bits(&self) -> &[bool] {
        &self.bits
    }
    pub fn selected_points(&self) -> impl Iterator<Item = [u32; 2]> + '_ {
        let width = self.size[0] as usize;
        self.bits
            .iter()
            .enumerate()
            .filter_map(move |(index, selected)| {
                selected.then_some([
                    self.origin[0] + (index % width) as u32,
                    self.origin[1] + (index / width) as u32,
                ])
            })
    }
    /// Exact support groups in the healer's donor order, for donor geometry presentation.
    pub fn grouped_supports(&self) -> Vec<Self> {
        let width = self.size[0] as usize;
        grouped_components(&self.bits, width, self.size[1] as usize)
            .into_iter()
            .map(|group| {
                let min_x = group.iter().map(|i| i % width).min().unwrap();
                let max_x = group.iter().map(|i| i % width).max().unwrap();
                let min_y = group.iter().map(|i| i / width).min().unwrap();
                let max_y = group.iter().map(|i| i / width).max().unwrap();
                let size = [(max_x - min_x + 1) as u32, (max_y - min_y + 1) as u32];
                let mut bits = vec![false; (size[0] * size[1]) as usize];
                for i in group {
                    bits[(i / width - min_y) * size[0] as usize + i % width - min_x] = true;
                }
                Self::new(
                    [self.origin[0] + min_x as u32, self.origin[1] + min_y as u32],
                    size,
                    bits,
                )
                .expect("validated nonempty donor group")
            })
            .collect()
    }

    pub fn contains(&self, point: [u32; 2]) -> bool {
        let [x, y] = point;
        if x < self.origin[0]
            || y < self.origin[1]
            || x >= self.origin[0] + self.size[0]
            || y >= self.origin[1] + self.size[1]
        {
            return false;
        }
        self.bits[((y - self.origin[1]) * self.size[0] + x - self.origin[0]) as usize]
    }
    fn intersects(&self, other: &Self) -> bool {
        let left = self.origin[0].max(other.origin[0]);
        let top = self.origin[1].max(other.origin[1]);
        let right = (self.origin[0] + self.size[0]).min(other.origin[0] + other.size[0]);
        let bottom = (self.origin[1] + self.size[1]).min(other.origin[1] + other.size[1]);
        (top..bottom)
            .any(|y| (left..right).any(|x| self.contains([x, y]) && other.contains([x, y])))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repair {
    pub id: RepairId,
    pub enabled: bool,
    pub origin: RepairOrigin,
    pub support: SupportMask,
    pub donor: DonorChoice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairSet {
    next_id: u64,
    repairs: Vec<Repair>,
    selected: Option<RepairId>,
}
impl RepairSet {
    pub fn repairs(&self) -> &[Repair] {
        &self.repairs
    }
    pub fn selected(&self) -> Option<RepairId> {
        self.selected
    }
    pub fn insert(
        &mut self,
        support: SupportMask,
        origin: RepairOrigin,
        donor: DonorChoice,
    ) -> Result<RepairId, Error> {
        self.require_independent(&support, None)?;
        let next = self
            .next_id
            .checked_add(1)
            .ok_or(Error::InvalidParameter("repair ID space exhausted"))?;
        let id = RepairId(next);
        self.next_id = next;
        self.repairs.push(Repair {
            id,
            enabled: true,
            origin,
            support,
            donor,
        });
        self.selected = Some(id);
        Ok(id)
    }
    pub fn select(&mut self, id: Option<RepairId>) -> Result<(), Error> {
        if id.is_some_and(|id| !self.repairs.iter().any(|repair| repair.id == id)) {
            return Err(Error::InvalidParameter("unknown repair ID"));
        }
        self.selected = id;
        Ok(())
    }
    pub fn remove(&mut self, id: RepairId) -> Result<Repair, Error> {
        let index = self
            .repairs
            .iter()
            .position(|repair| repair.id == id)
            .ok_or(Error::InvalidParameter("unknown repair ID"))?;
        let repair = self.repairs.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
        }
        Ok(repair)
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
    mut base: impl FnMut([u32; 2]) -> Result<Ap1, Error>,
) -> Result<EvaluatedRepair, Error> {
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
    let selected = repair
        .support
        .bits
        .iter()
        .enumerate()
        .filter_map(|(index, bit)| bit.then_some(index))
        .collect::<Vec<_>>();
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
    let (result, mut donors) =
        heal_ranked(&image, [window.width, window.height], &mask, repair.donor)?;
    for donor in &mut donors {
        donor.target[0] += window.x;
        donor.target[1] += window.y;
        donor.source[0] += window.x;
        donor.source[1] += window.y;
    }
    let mut values = vec![[0.; 3]; repair.support.bits.len()];
    for (index, selected) in repair.support.bits.iter().enumerate() {
        if *selected {
            let x = repair.support.origin[0] + (index % local_width) as u32;
            let y = repair.support.origin[1] + (index / local_width) as u32;
            values[index] = result[((y - window.y) * window.width + x - window.x) as usize];
        }
    }
    Ok(EvaluatedRepair {
        id: repair.id,
        support: repair.support.clone(),
        donors,
        values,
    })
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

/// Frozen boundary-ranked donor healing in signed/HDR AP1.
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
    let [width, height] = dimensions(size, image.len())?;
    if mask.len() != image.len() || mask.iter().all(|bit| *bit) {
        return Err(Error::InvalidParameter("invalid repair mask"));
    }
    if image.iter().flatten().any(|value| !value.is_finite()) {
        return Err(Error::InvalidParameter("repair requires finite AP1"));
    }
    if !mask.iter().any(|bit| *bit) {
        return Ok((image.to_vec(), Vec::new()));
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
        let donor_match = heal_component(
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
        )?;
        matches.push(donor_match);
    }
    if output.iter().flatten().any(|value| !value.is_finite())
        || output.iter().zip(image).zip(mask).any(|((a, b), hidden)| {
            !hidden && a.iter().zip(b).any(|(x, y)| x.to_bits() != y.to_bits())
        })
    {
        return Err(Error::NumericalOverflow);
    }
    Ok((output, matches))
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
) -> Result<DonorMatch, Error> {
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
    let distances = distance_to_unknown(&known, local_width, local_height);
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
            let mut candidates = Vec::new();
            let mut best = f64::INFINITY;
            let mut zero_winner = None;
            for sy in search_y0..=search_y1 - local_height {
                for sx in search_x0..=search_x1 - local_width {
                    if zero_winner
                        .is_some_and(|winner| !can_beat_zero_winner(winner, [sx, sy], [x0, y0]))
                    {
                        continue;
                    }
                    if eligibility.clean(sx, sy, search)
                        && let Ok(Some(score)) = matching_score_bounded(
                            MatchContext {
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
                            },
                            totals,
                            search.ceiling(best),
                        )
                    {
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
    let correction = limited_biharmonic(&difference, &local, local_width, local_height)?;
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
    Ok(DonorMatch {
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
    })
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
fn distance_to_unknown(known: &[bool], width: usize, _height: usize) -> Vec<f64> {
    let unknown = known
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (!value).then_some((index % width, index / width)))
        .collect::<Vec<_>>();
    known
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if !value {
                0.
            } else {
                let x = index % width;
                let y = index / width;
                unknown
                    .iter()
                    .map(|(ux, uy)| distance(x, y, *ux, *uy))
                    .fold(f64::INFINITY, f64::min)
            }
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
#[path = "dust_repair_tests.rs"]
mod tests;
