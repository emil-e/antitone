//! Authored healing is a nonlocal CPU image calculation over unrepaired neutral pixels.
use crate::gpu::evaluation::*;
use crate::{
    Error,
    dust_repair::{self, BaseImage, Repair, SupportMask},
    geometry::SourceRegion,
};
use std::sync::Arc;

pub(crate) fn compose(base: Value, repairs: &[Repair]) -> Result<Value, Error> {
    let replacements = repairs
        .iter()
        .filter(|repair| repair.enabled)
        .map(|repair| Ok((repair.support.clone(), healing_image(&base, repair)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    compose_images(base, replacements)
}

pub(crate) fn compose_images(
    base: Value,
    replacements: Vec<(SupportMask, Value)>,
) -> Result<Value, Error> {
    if base.domain().meaning != Meaning::Neutral
        || base.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
    {
        return Err(Error::InvalidParameter(
            "healing requires original neutral cells",
        ));
    }
    let active = base.domain().finite()?.source()?;
    if replacements.is_empty() {
        return Ok(base);
    }
    let mut inputs = vec![base.clone()];
    let mut supports: Vec<SupportMask> = Vec::new();
    for (support, image) in replacements {
        support.validate()?;
        let region = bounds(&support);
        if !active.contains_region(region)
            || supports.iter().any(|other| other.intersects(&support))
        {
            return Err(Error::InvalidParameter(
                "repair supports overlap or exceed the source",
            ));
        }
        if image.domain().meaning != Meaning::Neutral
            || !image.domain().same_grid(base.domain())
            || !image.domain().contains_region(region.try_into()?)
        {
            return Err(Error::InvalidParameter(
                "replacement image differs from its original neutral cells",
            ));
        }
        inputs.push(image);
        supports.push(support);
    }
    Ok(Value::from_operation(
        base.domain(),
        inputs,
        Compose { supports },
    ))
}

pub(crate) fn healing_image(base: &Value, repair: &Repair) -> Result<Value, Error> {
    if base.domain().meaning != Meaning::Neutral
        || base.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
    {
        return Err(Error::InvalidParameter(
            "healing requires original neutral cells",
        ));
    }
    repair.support.validate()?;
    let active = base.domain().finite()?.source()?;
    let bounds = bounds(&repair.support);
    if !active.contains_region(bounds) {
        return Err(Error::InvalidParameter(
            "repair supports overlap or exceed the source",
        ));
    }
    let region = bounds.try_into()?;
    let calculation = Heal {
        repair: repair.clone(),
        active,
    };
    let values = calculation.input_regions().and_then(|regions| {
        regions
            .into_iter()
            .map(|region| base.request(region)?.read(region))
            .collect::<Result<Vec<_>, Error>>()
    });
    let (values, operation) = match values {
        Ok(values) => (
            values,
            value::Operation::cpu(
                dust_repair::REPAIR_CONTRACT,
                calculation.identity(),
                Arc::new(calculation),
            ),
        ),
        Err(error) => (
            vec![base.request(region)?.read(region)?],
            value::Operation::failed(dust_repair::REPAIR_CONTRACT, calculation.identity(), error),
        ),
    };
    value::Value::new(
        Domain {
            bounds: Some(region),
            ..base.domain()
        },
        region,
        values,
        operation,
    )
}

fn bounds(support: &SupportMask) -> SourceRegion {
    let [x, y] = support.origin();
    let [width, height] = support.size();
    SourceRegion {
        x,
        y,
        width,
        height,
    }
}

struct Heal {
    repair: Repair,
    active: SourceRegion,
}
#[derive(Debug, PartialEq)]
struct HealingIdentity {
    contract: &'static str,
    support: SupportMask,
    offset: [i32; 2],
    active: SourceRegion,
}
impl Heal {
    fn identity(&self) -> HealingIdentity {
        HealingIdentity {
            contract: dust_repair::REPAIR_CONTRACT,
            support: self.repair.support.clone(),
            offset: self.repair.offset,
            active: self.active,
        }
    }

    fn input_regions(&self) -> Result<Vec<CellRegion>, Error> {
        dust_repair::input_regions(&self.repair, self.active)?
            .into_iter()
            .map(CellRegion::try_from)
            .collect()
    }
}
impl CpuKernel for Heal {
    fn run(
        &self,
        inputs: &[CpuImage],
        output: CellRegion,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        let mut base = Inputs(inputs);
        let mut execution = dust_repair::Execution { cancelled };
        let Some(evaluated) =
            dust_repair::evaluate_image(&self.repair, self.active, &mut base, &mut execution)?
        else {
            return Ok(None);
        };
        let mut pixels = vec![[0.; 4]; output.width as usize * output.height as usize];
        for (point, value) in evaluated.pixels() {
            if !value.iter().all(|value| value.is_finite()) {
                return Err(Error::NumericalOverflow);
            }
            pixels[output.index(point.map(|value| value as i32))] =
                [value[0], value[1], value[2], 0.];
        }
        Ok(Some(pixels))
    }
}
struct Inputs<'a>(&'a [CpuImage]);
impl BaseImage for Inputs<'_> {
    fn read(
        &mut self,
        region: SourceRegion,
        needed: Option<&[bool]>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<dust_repair::Ap1>>, Error> {
        let region: CellRegion = region.try_into()?;
        let input =
            self.0
                .iter()
                .find(|input| input.region == region)
                .ok_or(Error::InvalidParameter(
                    "healer requested an undeclared input",
                ))?;
        if needed.is_some_and(|mask| mask.len() != input.pixels.len()) {
            return Err(Error::InvalidParameter(
                "repair selection differs from its input",
            ));
        }
        let mut pixels = vec![[0.; 3]; input.pixels.len()];
        for (index, value) in input.pixels.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            if needed.is_none_or(|mask| mask[index]) {
                let rgb = [value[0], value[1], value[2]];
                if !rgb.iter().all(|value| value.is_finite()) {
                    return Err(Error::NumericalOverflow);
                }
                pixels[index] = rgb;
            }
        }
        Ok(Some(pixels))
    }
}

struct Compose {
    supports: Vec<SupportMask>,
}
impl Compose {
    fn influencing(
        &self,
        region: CellRegion,
    ) -> impl Iterator<Item = (usize, &SupportMask, CellRegion)> {
        self.supports
            .iter()
            .enumerate()
            .filter_map(move |(index, support)| {
                let patch = bounds(support);
                let origin = [region.x.max(patch.x as i32), region.y.max(patch.y as i32)];
                let end = [
                    region.right().min((patch.x + patch.width) as i32),
                    region.bottom().min((patch.y + patch.height) as i32),
                ];
                if end[0] <= origin[0] || end[1] <= origin[1] {
                    return None;
                }
                let selected = support.selected_points().any(|point| {
                    i64::from(point[0]) >= i64::from(origin[0])
                        && i64::from(point[0]) < i64::from(end[0])
                        && i64::from(point[1]) >= i64::from(origin[1])
                        && i64::from(point[1]) < i64::from(end[1])
                });
                if !selected {
                    return None;
                }
                Some((
                    index + 1,
                    support,
                    CellRegion::new(
                        origin,
                        [(end[0] - origin[0]) as u32, (end[1] - origin[1]) as u32],
                    ),
                ))
            })
    }
}
impl Operation for Compose {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.supports.clone()))
    }

    fn describe(
        &self,
        output: Output,
        inputs: &[crate::gpu::evaluation::value::Value],
        requested: &[CellRegion],
    ) -> Result<Output, Error> {
        let selected: Vec<_> = self
            .influencing(output.region)
            .filter(|(_, support, _)| {
                support.selected_points().any(|point| {
                    requested.iter().any(|read| {
                        i64::from(point[0]) >= i64::from(read.x)
                            && i64::from(point[0]) < i64::from(read.right())
                            && i64::from(point[1]) >= i64::from(read.y)
                            && i64::from(point[1]) < i64::from(read.bottom())
                    })
                })
            })
            .collect();
        if selected.is_empty() {
            return inputs[0].output(output.region);
        }
        Ok(Output::calculated(
            output.domain,
            output.region,
            Some(RetainedKey::new(
                selected
                    .iter()
                    .map(|(_, support, _)| (*support).clone())
                    .collect::<Vec<_>>(),
            )),
            std::iter::once(inputs[0].content()).chain(
                selected
                    .iter()
                    .map(|(index, _, _)| inputs[*index].content()),
            ),
        ))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(std::iter::once(Read::new(0, output))
            .chain(
                self.influencing(output)
                    .map(|(index, _, region)| Read::new(index, region)),
            )
            .collect())
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        self.express(&inputs, domain, output).map(Some)
    }
}

mod callable;

#[cfg(all(test, feature = "editor"))]
mod tests;
