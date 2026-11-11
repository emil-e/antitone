//! Related declared units share phase inputs, not an operation-specific evaluation loop.
use super::gpu::GpuExecution;
use super::unit::{matching_unit, wait_image};
use super::*;

type UnitResult = Result<Arc<ImageValue>, Error>;

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(super) struct Target<'a> {
    pub(super) evaluation: &'a Evaluation,
    pub(super) layout: ImageLayout,
    pub(super) unit: Option<WorkUnit>,
}
struct Running {
    layout: ImageLayout,
    execution: GpuExecution,
    output: Arc<ImageValue>,
    phase: Option<GpuPhase>,
    finished: bool,
    failure: Option<Error>,
}

impl Worker<'_> {
    pub(super) fn materialize_units_many(
        &self,
        jobs: &[&(Evaluation, ImageLayout)],
    ) -> Result<Option<Vec<Arc<ImageValue>>>, Error> {
        if jobs.len() < 2 {
            return Ok(None);
        }
        // Related roots with a dependency on each other must publish in DAG order;
        // their nested execution already uses ordinary inline claims.
        for (i, (evaluation, _)) in jobs.iter().enumerate() {
            for (j, (other, _)) in jobs.iter().enumerate() {
                if i != j && depends_on(evaluation.job.image(), other.job.image()) {
                    return Ok(None);
                }
            }
        }
        let budget = jobs
            .iter()
            .map(|(evaluation, _)| evaluation.job.budget().scratch_bytes)
            .min()
            .unwrap();
        for (evaluation, layout) in jobs {
            if Stored::find(Some(self.scope), &[], evaluation.job.image(), layout.region)?.is_some()
            {
                return Ok(None);
            }
            if super::super::unit::can_inline(
                evaluation.job.image(),
                layout.region,
                evaluation.job.budget(),
            )? {
                let plan = evaluation.job.probe(layout.region)?;
                let resident = plan.peak_scratch_bytes <= evaluation.job.budget().scratch_bytes
                    && Executor::admit(&self.context.device().limits(), &plan).is_ok()
                    && !plan
                        .tiles
                        .iter()
                        .flat_map(|tile| &tile.fragment.work)
                        .any(|work| matches!(work, Work::Upload { .. } | Work::Produce { .. }));
                if resident {
                    return Ok(None);
                }
            }
        }
        let mut held = Vec::new();
        for (evaluation, layout) in jobs {
            let Some(unit) = matching_unit(
                evaluation.job.image(),
                layout.region,
                evaluation.job.budget(),
            )?
            else {
                return Ok(None);
            };
            let unit =
                match unit.admit_for(evaluation.job.budget(), &self.context.device().limits()) {
                    Ok(unit) => unit,
                    Err(Error::WorkingAllowance(_)) => return Ok(None),
                    Err(error) => return Err(error),
                };
            let UnitWork::Gpu(work) = unit.work else {
                return Ok(None);
            };
            if unit.output != layout.region || work.state.is_empty() {
                return Ok(None);
            }
            let bytes = work
                .state
                .iter()
                .try_fold(layout.bytes(), |sum, bytes| sum.checked_add(*bytes))
                .ok_or(Error::InvalidParameter("unit state size overflows"))?;
            held.push(bytes);
        }
        let Some(total) = held
            .iter()
            .try_fold(0u64, |sum, bytes| sum.checked_add(*bytes))
        else {
            return Ok(None);
        };
        // Leave ample input/assembly space. Large units run independently instead of
        // letting batch concurrency turn an individually admitted job into a failure.
        if total >= budget / 4 {
            return Ok(None);
        }
        let mut targets = Vec::new();
        for ((evaluation, layout), own) in jobs.iter().copied().zip(&held) {
            let unit_budget = Budget {
                scratch_bytes: budget - (total - own) - budget / 4,
                ..evaluation.job.budget()
            };
            let Some(unit) = matching_unit(evaluation.job.image(), layout.region, unit_budget)?
            else {
                return Ok(None);
            };
            let UnitWork::Gpu(work) = &unit.work else {
                return Ok(None);
            };
            if unit.output != layout.region
                || work.state.iter().sum::<u64>() + layout.bytes() != *own
            {
                return Ok(None);
            }
            targets.push(Target {
                evaluation,
                layout: *layout,
                unit: Some(unit),
            });
        }
        self.materialize_targets(targets, total, budget)
    }

    pub(super) fn materialize_targets(
        &self,
        mut targets: Vec<Target<'_>>,
        total: u64,
        budget: u64,
    ) -> Result<Option<Vec<Arc<ImageValue>>>, Error> {
        let mut claims = Vec::new();
        for target in &targets {
            let node = target.evaluation.job.image();
            let Some(identity) = node.output(target.layout.region)?.identity().cloned() else {
                return Ok(None);
            };
            claims.push(self.scope.claim_unit(
                identity.clone(),
                Arc::new(target.layout),
                target.evaluation.job.budget(),
            ));
        }
        let cancelled = || {
            (self.cancelled)()
                && claims
                    .iter()
                    .filter_map(|(_, producer)| producer.as_ref())
                    .all(|producer| producer.cancel_when_unshared())
        };
        let worker = Worker {
            budget: Budget {
                scratch_bytes: budget,
                ..self.budget
            },
            cancelled: &cancelled,
            ..*self
        };
        let mut counts = WorkCounts::default();
        let calculated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&worker, &mut targets, &claims, total, &mut counts)
        }))
        .unwrap_or_else(|_| {
            claims
                .iter()
                .map(|_| Some(Err(Error::Gpu("related unit worker panicked".into()))))
                .collect()
        });
        // Retire shared state before retrying an aggregate admission failure under
        // that root's original allowance. Genuine calculation failures stay local.
        let results = targets
            .into_iter()
            .zip(&claims)
            .zip(calculated)
            .map(|((target, (_, producer)), result)| {
                if producer.is_none() {
                    return Ok(None);
                }
                match result {
                    Some(Err(Error::WorkingAllowance(_))) => {
                        let budget = target.evaluation.job.budget();
                        let unit = matching_unit(
                            target.evaluation.job.image(),
                            target.layout.region,
                            budget,
                        )?
                        .ok_or(Error::InvalidParameter("batch unit changed its coverage"))?;
                        Worker { budget, ..worker }
                            .compute_unit(target.evaluation.job.image(), unit)
                            .map(|(image, _)| Some(image))
                    }
                    Some(result) => result.map(Some),
                    None => Err(Error::InvalidParameter("batch omitted a claimed unit")),
                }
            })
            .collect::<Vec<_>>();
        // Release the cancellation borrow before consuming the physical producers.
        let mut pending = Vec::new();
        for ((future, producer), result) in claims.into_iter().zip(results) {
            if let Some(producer) = producer {
                producer.complete(result.and_then(|image| {
                    image.ok_or(Error::InvalidParameter("batch omitted a claimed unit"))
                }));
            }
            pending.push(future);
        }
        pending
            .into_iter()
            .map(|future| {
                wait_image(future, self.cancelled, self.progress).and_then(|image| {
                    super::super::materialized::validate(
                        image.descriptor.domain,
                        *image.descriptor,
                        &image.image,
                    )?;
                    Ok(image.image)
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

fn run(
    worker: &Worker<'_>,
    targets: &mut [Target<'_>],
    claims: &[(
        ImageFuture<ImageLayout>,
        Option<super::super::materializations::InlineProducer>,
    )],
    total: u64,
    counts: &mut WorkCounts,
) -> Vec<Option<UnitResult>> {
    let mut running = Vec::new();
    let mut failed = vec![None; targets.len()];
    for (i, (target, (_, producer))) in targets.iter_mut().zip(claims).enumerate() {
        if producer.is_none() {
            running.push(None);
            continue;
        }
        let unit = target.unit.take().unwrap();
        let UnitWork::Gpu(work) = unit.work else {
            unreachable!()
        };
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::evaluation_buffer_created(target.layout.bytes());
        let output = worker
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("related declared unit"),
                size: target.layout.bytes(),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        counts.buffers_allocated(1);
        let execution = match GpuExecution::new(
            worker,
            unit.inputs,
            target.layout.region,
            work,
            &output,
            counts,
        ) {
            Ok(execution) => execution,
            Err(error) => {
                failed[i] = Some(error);
                running.push(None);
                continue;
            }
        };
        running.push(Some(Running {
            layout: target.layout,
            execution,
            output: ImageValue::new(output),
            phase: None,
            finished: false,
            failure: None,
        }));
    }
    loop {
        for target in running
            .iter_mut()
            .flatten()
            .filter(|target| !target.finished && target.phase.is_none())
        {
            match target.execution.phases.next() {
                Some(Ok(phase)) => target.phase = Some(phase),
                Some(Err(error)) => {
                    target.failure = Some(error);
                    target.finished = true;
                }
                None => {
                    target.failure = target.execution.finish().err();
                    target.finished = true;
                }
            }
        }
        let next = running
            .iter()
            .flatten()
            .filter_map(|target| target.phase.as_ref())
            .map(order)
            .min();
        let Some(next) = next else {
            break;
        };
        let cover = running
            .iter()
            .flatten()
            .filter_map(|target| target.phase.as_ref())
            .find(|phase| order(phase) == next)
            .and_then(|phase| phase.reads.first())
            .map(|read| read.region);
        let mut owners = Vec::new();
        let mut indices: Vec<_> = running
            .iter()
            .enumerate()
            .filter_map(|(i, target)| {
                target
                    .as_ref()
                    .and_then(|target| target.phase.as_ref())
                    .map(|phase| (order(phase), i))
            })
            .collect();
        indices.sort_by_key(|(order, _)| *order);
        for (_, i) in indices {
            let target = running[i].as_mut().unwrap();
            let phase = target.phase.as_ref().unwrap();
            let selected = match (cover, phase.reads.first()) {
                (None, None) => true,
                (Some(cover), Some(read)) => cover.contains_region(read.region),
                _ => false,
            };
            if selected {
                let phase = target.phase.take().unwrap();
                if let Err(error) = target.execution.execute(
                    worker,
                    phase,
                    total - target.execution.held,
                    &mut owners,
                    counts,
                ) {
                    target.failure = Some(error);
                    target.finished = true;
                }
            }
        }
        // Every selected phase has completed its submission. No source chunk owner
        // survives the round merely to offer speculative whole-job reuse.
    }
    running
        .into_iter()
        .zip(failed)
        .map(|(target, failed)| {
            target
                .map(|target| {
                    debug_assert_eq!(target.layout.backing, ImageBacking::Buffer);
                    target.failure.map_or(Ok(target.output), Err)
                })
                .or_else(|| failed.map(Err))
        })
        .collect()
}

fn order(phase: &GpuPhase) -> (bool, i32, i32, std::cmp::Reverse<u64>) {
    match phase.reads.first() {
        None => (false, 0, 0, std::cmp::Reverse(0)),
        Some(read) => (
            true,
            read.region.y,
            read.region.x,
            std::cmp::Reverse(u64::from(read.region.width) * u64::from(read.region.height)),
        ),
    }
}
fn depends_on(node: &value::Value, other: &value::Value) -> bool {
    node.inputs().iter().any(|input| {
        input.same_capture(other)
            || input.identity().is_some() && input.identity() == other.identity()
            || depends_on(input, other)
    })
}
