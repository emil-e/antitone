//! Bounded work units: logical reads and computation, without upstream evaluation.
use super::*;

pub type WorkUnits = Box<dyn Iterator<Item = CellRegion> + Send>;

pub(super) enum Lowering {
    Source(Arc<dyn Source>),
    Gpu {
        phase: GpuPhase,
        inputs: Vec<value::Value>,
    },
    Deferred,
}

pub(super) fn plan(
    node: &value::Value,
    output: CellRegion,
    budget: Budget,
) -> Result<WorkUnit, Error> {
    let calculation = calculation(node, output, budget)?;
    calculation.compile(output, budget)
}

pub(super) fn calculation(
    node: &value::Value,
    output: CellRegion,
    budget: Budget,
) -> Result<value::Calculation, Error> {
    if output.width.checked_mul(output.height).is_none() {
        return Err(Error::InvalidParameter(
            "evaluation raster exceeds GPU cell addressing",
        ));
    }
    let calculation = node.lower(output, budget)?;
    if calculation.shape() != value::Shape::Pixel {
        return Err(Error::InvalidParameter(
            "record values cannot be requested as photographic images",
        ));
    }
    Ok(calculation)
}

/// The GPU compiler can combine a single stateless phase; other units execute separately.
pub(super) fn lowering(
    node: &value::Value,
    region: CellRegion,
    budget: Budget,
) -> Result<Lowering, Error> {
    if !single_unit(node, region, budget)? {
        return Ok(Lowering::Deferred);
    }
    let first = region;
    let calculation = match calculation(node, first, budget) {
        Ok(calculation) => calculation,
        Err(Error::WorkingAllowance(_)) => return Ok(Lowering::Deferred),
        Err(error) => return Err(error),
    };
    lowered(calculation, region, budget)
}

pub(super) fn lowered(
    calculation: value::Calculation,
    region: CellRegion,
    budget: Budget,
) -> Result<Lowering, Error> {
    let domain = calculation.domain();
    if let Some(alias) = calculation.value() {
        if !single_unit(alias, region, budget)? {
            return Ok(Lowering::Deferred);
        }
        let expanded = self::calculation(alias, region, budget)?;
        if !expanded
            .value()
            .is_some_and(|value| value.same_capture(alias))
        {
            return lowered(expanded, region, budget);
        }
    }
    if calculation
        .inputs()
        .iter()
        .any(|input| input.shape() != value::Shape::Pixel)
    {
        return Ok(Lowering::Deferred);
    }
    let Ok(unit) = calculation.plan_unit(region, budget) else {
        return Ok(Lowering::Deferred);
    };
    if unit.domain != domain || unit.output != region {
        return Ok(Lowering::Deferred);
    }
    Ok(match unit.work {
        UnitWork::Source(source) => Lowering::Source(source),
        UnitWork::Cpu(_) => Lowering::Deferred,
        UnitWork::Gpu(mut work) => {
            if !work.state.is_empty() {
                return Ok(Lowering::Deferred);
            }
            let phase = work
                .phases
                .next()
                .ok_or(Error::InvalidParameter("GPU unit has no phases"))??;
            if work.phases.next().transpose()?.is_some() {
                Lowering::Deferred
            } else {
                Lowering::Gpu {
                    phase,
                    inputs: unit.inputs,
                }
            }
        }
    })
}

pub(super) fn single_unit(
    node: &value::Value,
    region: CellRegion,
    budget: Budget,
) -> Result<bool, Error> {
    let mut units = node.operation.work_units(region, budget)?;
    let first = units
        .next()
        .ok_or(Error::InvalidParameter("stage declared no work units"))?;
    Ok(first == region && units.next().is_none())
}

pub(super) fn can_inline(
    node: &value::Value,
    region: CellRegion,
    budget: Budget,
) -> Result<bool, Error> {
    if !single_unit(node, region, budget)? {
        return Ok(false);
    }
    if matches!(lowering(node, region, budget)?, Lowering::Gpu { .. }) {
        return Ok(true);
    }
    match calculation(node, region, budget) {
        Ok(calculation) => Ok(value::FunctionGraph::new(&calculation)?.is_some()),
        Err(Error::WorkingAllowance(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn bisect(region: CellRegion) -> Option<[CellRegion; 2]> {
    let axis = usize::from(region.height > region.width);
    let size = [region.width, region.height][axis];
    if size <= 1 {
        return None;
    }
    let first = size / 2;
    let mut halves = [region; 2];
    if axis == 0 {
        halves[0].width = first;
        halves[1].x += first as i32;
        halves[1].width -= first;
    } else {
        halves[0].height = first;
        halves[1].y += first as i32;
        halves[1].height -= first;
    }
    Some(halves)
}

pub fn tiled_units(region: CellRegion, size: [u32; 2]) -> Result<WorkUnits, Error> {
    region.validate()?;
    if size.contains(&0) {
        return Err(Error::InvalidParameter(
            "work-unit dimensions must be positive",
        ));
    }
    Ok(Box::new(
        (region.y..region.bottom())
            .step_by(size[1] as usize)
            .flat_map(move |y| {
                (region.x..region.right())
                    .step_by(size[0] as usize)
                    .map(move |x| {
                        CellRegion::new(
                            [x, y],
                            [
                                size[0].min((region.right() - x) as u32),
                                size[1].min((region.bottom() - y) as u32),
                            ],
                        )
                    })
            }),
    ))
}

/// An assembled logical input. Physical upstream tiling is not exposed to CPU algorithms.
pub struct CpuImage {
    pub domain: Domain,
    pub region: CellRegion,
    pub pixels: Vec<[f32; 4]>,
}

/// CPU computation receives its declared inputs, never an upstream evaluator.
pub trait CpuKernel: Send + Sync {
    fn run(
        &self,
        inputs: &[CpuImage],
        output: CellRegion,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error>;
}

/// A phase binding names a logical read, persistent unit state, or the unit's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitValue {
    Read(usize),
    State(usize),
    Output,
}

/// GPU lowering receives admitted phase bindings, not an upstream evaluator.
pub trait GpuKernel: std::any::Any + Send + Sync {
    fn working_bytes(&self) -> u64 {
        0
    }
    fn lower(&self, fragment: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, Error>;
}

pub struct GpuPhase {
    pub reads: Vec<Read>,
    pub inputs: Vec<UnitValue>,
    pub kernel: Arc<dyn GpuKernel>,
}

/// Phase descriptions are lazy; state survives between phases, source inputs need not.
pub struct GpuUnit {
    pub state: Vec<u64>,
    pub phases: Box<dyn Iterator<Item = Result<GpuPhase, Error>> + Send>,
}

pub struct CpuUnit {
    pub reads: Vec<Read>,
    pub kernel: Arc<dyn CpuKernel>,
}

pub enum UnitWork {
    Source(Arc<dyn Source>),
    Gpu(GpuUnit),
    Cpu(CpuUnit),
}

/// One stage-selected computation. Its output may cover more than the caller requested.
pub struct WorkUnit {
    pub inputs: Vec<value::Value>,
    pub domain: Domain,
    pub output: CellRegion,
    pub work: UnitWork,
}

impl WorkUnit {
    pub(super) fn admit(mut self, budget: Budget) -> Result<Self, Error> {
        if let UnitWork::Gpu(work) = &mut self.work {
            let state = work
                .state
                .iter()
                .try_fold(0u64, |sum, bytes| sum.checked_add(*bytes))
                .ok_or(Error::InvalidParameter("unit state size overflows"))?;
            let output = u64::from(self.output.width) * u64::from(self.output.height) * 16;
            if state
                .checked_add(output)
                .is_none_or(|bytes| bytes >= budget.scratch_bytes)
            {
                return Err(Error::WorkingAllowance(
                    "unit state exceeds the working allowance",
                ));
            }
            let phases = std::mem::replace(&mut work.phases, Box::new(std::iter::empty()))
                .collect::<Result<Vec<_>, Error>>()?;
            for phase in &phases {
                let mut unique = Vec::<(&value::Value, CellRegion)>::new();
                let mut bytes = state
                    .checked_add(output)
                    .and_then(|bytes| bytes.checked_add(phase.kernel.working_bytes()))
                    .ok_or(Error::InvalidParameter("unit state size overflows"))?;
                for read in &phase.reads {
                    let input = self
                        .inputs
                        .get(read.input)
                        .ok_or(Error::InvalidParameter("read references an unknown input"))?;
                    read.region.validate()?;
                    if !input.domain().contains_region(read.region) {
                        return Err(Error::InvalidParameter("unit read exceeds upstream image"));
                    }
                    if !unique
                        .iter()
                        .any(|(prior, region)| prior.same_capture(input) && *region == read.region)
                    {
                        let cells = read.region.width.checked_mul(read.region.height).ok_or(
                            Error::InvalidParameter(
                                "evaluation raster exceeds GPU cell addressing",
                            ),
                        )?;
                        bytes = bytes
                            .checked_add(u64::from(cells) * 16)
                            .ok_or(Error::InvalidParameter("unit state size overflows"))?;
                        unique.push((input, read.region));
                    }
                }
                if bytes >= budget.scratch_bytes {
                    return Err(Error::WorkingAllowance(
                        "phase reads exceed the working allowance",
                    ));
                }
            }
            work.phases = Box::new(phases.into_iter().map(Ok));
        }
        Ok(self)
    }

    pub(super) fn admit_for(self, budget: Budget, limits: &wgpu::Limits) -> Result<Self, Error> {
        let mut unit = self.admit(budget)?;
        if let UnitWork::Gpu(work) = &mut unit.work {
            let maximum = limits
                .max_buffer_size
                .min(limits.max_storage_buffer_binding_size);
            if u64::from(unit.output.width) * u64::from(unit.output.height) * 16 > maximum
                || work.state.iter().any(|bytes| *bytes > maximum)
            {
                return Err(Error::WorkingAllowance(
                    "GPU unit exceeds device storage limits",
                ));
            }
            let phases = std::mem::replace(&mut work.phases, Box::new(std::iter::empty()))
                .collect::<Result<Vec<_>, _>>()?;
            if phases.iter().flat_map(|phase| &phase.reads).any(|read| {
                u64::from(read.region.width) * u64::from(read.region.height) * 16 > maximum
            }) {
                return Err(Error::WorkingAllowance(
                    "GPU unit exceeds device storage limits",
                ));
            }
            work.phases = Box::new(phases.into_iter().map(Ok));
        }
        Ok(unit)
    }
}
