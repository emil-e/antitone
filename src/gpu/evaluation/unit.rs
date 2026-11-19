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
    budget: Tiling,
) -> Result<WorkUnit, Error> {
    plan_with_limits(node, output, budget, None)
}

pub(super) fn plan_with_limits(
    node: &value::Value,
    output: CellRegion,
    budget: Tiling,
    limits: Option<&wgpu::Limits>,
) -> Result<WorkUnit, Error> {
    let calculation = calculation_with_limits(node, output, budget, limits)?;
    calculation.compile_with_limits(output, budget, limits)
}

pub(super) fn calculation(
    node: &value::Value,
    output: CellRegion,
    budget: Tiling,
) -> Result<value::Calculation, Error> {
    calculation_with_limits(node, output, budget, None)
}

pub(super) fn calculation_with_limits(
    node: &value::Value,
    output: CellRegion,
    budget: Tiling,
    limits: Option<&wgpu::Limits>,
) -> Result<value::Calculation, Error> {
    if output.width.checked_mul(output.height).is_none() {
        return Err(Error::InvalidParameter(
            "evaluation raster exceeds GPU cell addressing",
        ));
    }
    let calculation = match limits {
        Some(_) => node.lower_with_limits(output, budget, limits)?,
        None => node.lower(output, budget)?,
    };
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
    budget: Tiling,
    limits: Option<&wgpu::Limits>,
) -> Result<Lowering, Error> {
    if !single_unit(node, region, budget)? {
        return Ok(Lowering::Deferred);
    }
    let first = region;
    let calculation = match calculation_with_limits(node, first, budget, limits) {
        Ok(calculation) => calculation,
        Err(Error::EvaluationLimit(_)) => return Ok(Lowering::Deferred),
        Err(error) => return Err(error),
    };
    lowered(calculation, region, budget, limits)
}

pub(super) fn lowered(
    calculation: value::Calculation,
    region: CellRegion,
    budget: Tiling,
    limits: Option<&wgpu::Limits>,
) -> Result<Lowering, Error> {
    let domain = calculation.domain();
    if let Some(alias) = calculation.value() {
        if !single_unit(alias, region, budget)? {
            return Ok(Lowering::Deferred);
        }
        let expanded = calculation_with_limits(alias, region, budget, limits)?;
        if !expanded
            .value()
            .is_some_and(|value| value.same_capture(alias))
        {
            return lowered(expanded, region, budget, limits);
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
    budget: Tiling,
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
    budget: Tiling,
    limits: Option<&wgpu::Limits>,
) -> Result<bool, Error> {
    if !single_unit(node, region, budget)? {
        return Ok(false);
    }
    if matches!(
        lowering(node, region, budget, limits)?,
        Lowering::Gpu { .. }
    ) {
        return Ok(true);
    }
    match calculation_with_limits(node, region, budget, limits) {
        Ok(calculation) => Ok(value::FunctionGraph::new(&calculation)?.is_some()),
        Err(Error::EvaluationLimit(_)) => Ok(false),
        Err(error) => Err(error),
    }
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
    pub(super) fn admit_for(mut self, limits: &wgpu::Limits) -> Result<Self, Error> {
        let unit = &mut self;
        if let UnitWork::Gpu(work) = &mut unit.work {
            let maximum = limits
                .max_buffer_size
                .min(limits.max_storage_buffer_binding_size);
            if u64::from(unit.output.width) * u64::from(unit.output.height) * 16 > maximum
                || work.state.iter().any(|bytes| *bytes > maximum)
            {
                return Err(Error::EvaluationLimit(
                    "GPU unit exceeds device storage limits",
                ));
            }
            let phases = std::mem::replace(&mut work.phases, Box::new(std::iter::empty()))
                .collect::<Result<Vec<_>, _>>()?;
            for read in phases.iter().flat_map(|phase| &phase.reads) {
                let input = unit
                    .inputs
                    .get(read.input)
                    .ok_or(Error::InvalidParameter("read references an unknown input"))?;
                read.region.validate()?;
                if !input.domain().contains_region(read.region) {
                    return Err(Error::InvalidParameter("unit read exceeds upstream image"));
                }
                let cells = read.region.width.checked_mul(read.region.height).ok_or(
                    Error::InvalidParameter("evaluation raster exceeds GPU cell addressing"),
                )?;
                if u64::from(cells) * 16 > maximum {
                    return Err(Error::EvaluationLimit(
                        "GPU unit exceeds device storage limits",
                    ));
                }
            }
            work.phases = Box::new(phases.into_iter().map(Ok));
        }
        Ok(self)
    }
}
