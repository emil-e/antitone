//! Deterministic reconstruction from an accepted source-coordinate donor offset.

#[cfg(feature = "gpu")]
use crate::dust_repair::context::target_region;
use crate::dust_repair::{
    Ap1, BaseImage, DonorMatch, Execution, Repair, RepairId, RepairOrigin, SupportMask,
    context::{NearSupport, SearchTarget, build_search_target, describe_donor, fetch_crop},
    validate_offset,
};
use crate::{Error, geometry::SourceRegion};

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
        let [x, y] = self.support.origin();
        let [width, _] = self.support.size();
        let index = ((point[1] - y) * width + point[0] - x) as usize;
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
    evaluate_image(repair, active, &mut base, execution)
}

pub(crate) fn evaluate_image(
    repair: &Repair,
    active: SourceRegion,
    base: &mut impl BaseImage,
    execution: &mut Execution<'_>,
) -> Result<Option<EvaluatedRepair>, Error> {
    if (execution.cancelled)() {
        return Ok(None);
    }
    active.validate()?;
    let [support_x, support_y] = repair.support.origin();
    let [support_width, support_height] = repair.support.size();
    let support_right = support_x
        .checked_add(support_width)
        .ok_or(Error::InvalidParameter(
            "repair support coordinates overflow",
        ))?;
    let support_bottom = support_y
        .checked_add(support_height)
        .ok_or(Error::InvalidParameter(
            "repair support coordinates overflow",
        ))?;
    if support_x < active.x
        || support_y < active.y
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
    let mut values = vec![[0.; 3]; (support_width * support_height) as usize];
    let mut donors = Vec::new();
    for group in &groups {
        if (execution.cancelled)() {
            return Ok(None);
        }
        let Some(target) = build_search_target(&repair.support, group, active, base, execution)?
        else {
            continue;
        };
        let Some(donor) = fetch_crop(base, target.origin, target.size, repair.offset, execution)?
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
            "biharmonic repair requires a two-pixel exterior",
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
    let [support_x, support_y] = support.origin();
    let [support_width, _] = support.size();
    let mut component = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if target.local[index] {
                let point = [target.origin[0] + x as u32, target.origin[1] + y as u32];
                let support_index =
                    ((point[1] - support_y) * support_width + point[0] - support_x) as usize;
                component.push((
                    support_index,
                    std::array::from_fn(|channel| {
                        donor[index][channel] + correction[index][channel]
                    }),
                ));
            }
        }
    }
    let Some(donor_match) = describe_donor(target, support, near, offset, donor, execution) else {
        return Ok(None);
    };
    Ok(Some((donor_match, component)))
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

/// Rectangular target and donor inputs for the same grouping used by the healer.
#[cfg(feature = "gpu")]
pub(crate) fn input_regions(
    repair: &Repair,
    active: SourceRegion,
) -> Result<Vec<SourceRegion>, Error> {
    active.validate()?;
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
    let mut regions = Vec::new();
    for group in repair.support.grouped_supports() {
        if let Some(layout) = target_region(&group, active) {
            let target = layout.region;
            let donor = SourceRegion {
                x: (i64::from(target.x) + i64::from(repair.offset[0]))
                    .try_into()
                    .map_err(|_| Error::InvalidParameter("donor coordinates overflow"))?,
                y: (i64::from(target.y) + i64::from(repair.offset[1]))
                    .try_into()
                    .map_err(|_| Error::InvalidParameter("donor coordinates overflow"))?,
                ..target
            };
            if !active.contains_region(target) || !active.contains_region(donor) {
                return Err(Error::InvalidParameter(
                    "repair input is outside the source",
                ));
            }
            regions.extend([target, donor]);
        }
    }
    Ok(regions)
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
