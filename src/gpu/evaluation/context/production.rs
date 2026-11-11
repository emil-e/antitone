//! Worker execution capabilities, separate from the image definition and optional retention.
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone, Copy)]
pub struct Worker<'a> {
    pub context: &'a Context,
    pub queue: &'a wgpu::Queue,
    pub gate: &'a crate::gpu::submission::SubmissionGate,
    pub scope: &'a Materializations,
    pub budget: Budget,
    pub working: &'a HostWorkingSet,
    pub cancelled: &'a dyn Fn() -> bool,
    pub progress: &'a dyn Fn(),
}

pub struct Production<'a> {
    pub worker: Worker<'a>,
    pub layout: ImageLayout,
    pub pixels: &'a wgpu::Buffer,
}
impl<'a> std::ops::Deref for Production<'a> {
    type Target = Worker<'a>;
    fn deref(&self) -> &Worker<'a> {
        &self.worker
    }
}

pub(super) fn admitted_plans<'a>(
    evaluation: Evaluation,
    region: CellRegion,
    budget: Budget,
    limits: wgpu::Limits,
    (cancelled, progress): (&'a dyn Fn() -> bool, &'a dyn Fn()),
) -> Result<impl Iterator<Item = Result<Plan, Error>> + 'a, Error> {
    let mut regions = tiled_units(region, budget.tile)?;
    let mut pending = Vec::new();
    Ok(std::iter::from_fn(move || {
        loop {
            let region = pending.pop().or_else(|| regions.next())?;
            progress();
            if cancelled() {
                return Some(Err(Error::Cancelled));
            }
            let candidate = (|| {
                let mut plan = evaluation.job.probe(region)?;
                if plan.requires_production(evaluation.job.image())? {
                    super::super::unit::plan(evaluation.job.image(), region, budget)?
                        .admit_for(budget, &limits)?;
                }
                Executor::admit(&limits, &plan)?;
                if plan.peak_scratch_bytes > budget.scratch_bytes {
                    return Err(Error::WorkingAllowance(
                        "evaluation neighbourhood exceeds working storage",
                    ));
                }
                plan.scratch_budget = budget.scratch_bytes;
                Ok(plan)
            })();
            match candidate {
                Ok(plan) => return Some(Ok(plan)),
                Err(error @ Error::WorkingAllowance(_)) => {
                    let Some([first, second]) = super::super::unit::bisect(region) else {
                        return Some(Err(error));
                    };
                    pending.extend([second, first]);
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }))
}

impl Production<'_> {
    pub(crate) fn produce(&self, evaluation: &Evaluation) -> Result<WorkCounts, Error> {
        let evaluation = evaluation.clone().with_materializations(self.scope.clone());
        let node = evaluation.job.image();
        let stored = super::super::materialized::Stored::find(
            Some(self.scope),
            &[],
            node,
            self.layout.region,
        )?
        .is_some();
        if !stored && super::super::unit::can_inline(node, self.layout.region, self.budget)? {
            let plan = evaluation.job.probe(self.layout.region)?;
            let deferred = plan.requires_production(node)?
                || plan.peak_scratch_bytes > self.budget.scratch_bytes
                || Executor::admit(&self.context.device().limits(), &plan).is_err();
            return if deferred {
                self.produce_units(node)
            } else {
                self.deliver(&mut self.executor(), plan)
            };
        }
        if !stored
            && matches!(
                super::super::unit::lowering(node, self.layout.region, self.budget)?,
                super::super::unit::Lowering::Deferred
            )
        {
            return self.produce_units(node);
        }
        self.evaluate(&evaluation)
    }

    pub(super) fn evaluate(&self, evaluation: &Evaluation) -> Result<WorkCounts, Error> {
        let mut executor = self.executor();
        let mut stats = WorkCounts::default();
        for plan in admitted_plans(
            evaluation.clone(),
            self.layout.region,
            self.budget,
            self.context.device().limits(),
            (self.cancelled, self.progress),
        )? {
            stats.include(self.deliver(&mut executor, plan?)?);
        }
        Ok(stats)
    }

    pub(in crate::gpu::evaluation) fn run_sink(
        &self,
        executor: &mut Executor,
        plan: &Plan,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
        self.context.run_sink(
            executor,
            (self.queue, self.gate),
            plan,
            (self.cancelled, self.progress),
            Sink {
                encode: sink,
                submitted: || {},
            },
        )
    }

    pub(in crate::gpu::evaluation) fn deliver(
        &self,
        executor: &mut Executor,
        mut plan: Plan,
    ) -> Result<WorkCounts, Error> {
        (self.progress)();
        if (self.cancelled)() {
            return Err(Error::Cancelled);
        }
        plan.scratch_budget = self.budget.scratch_bytes;
        let resident = plan.region == self.layout.region
            && !plan
                .tiles
                .iter()
                .flat_map(|tile| &tile.fragment.work)
                .any(|work| matches!(work, Work::Upload { .. } | Work::Produce { .. }));
        if resident {
            let recorded = self.executor().prepare(self.context.device(), plan)?;
            let mut encoder = self
                .context
                .device()
                .create_command_encoder(&Default::default());
            let mut stats =
                recorded.encode_result_counts(self.context.device(), &mut encoder, self.pixels)?;
            stats.include(recorded.preparation.clone());
            self.context
                .submit_with(self.queue, self.gate, encoder, self.cancelled, || {
                    recorded.submitted()
                })?
                .ok_or(Error::Cancelled)?;
            stats.submitted();
            Ok(stats)
        } else {
            self.run_sink(executor, &plan, |encoder, part, input| {
                for row in 0..part.region.height {
                    let destination = ((u64::from((part.region.y - self.layout.region.y) as u32)
                        + u64::from(row))
                        * u64::from(self.layout.region.width)
                        + u64::from((part.region.x - self.layout.region.x) as u32))
                        * 16;
                    encoder.copy_buffer_to_buffer(
                        input,
                        u64::from(row) * u64::from(part.region.width) * 16,
                        self.pixels,
                        destination,
                        u64::from(part.region.width) * 16,
                    );
                }
                Ok(())
            })?
            .ok_or(Error::Cancelled)
        }
    }
}

impl Worker<'_> {
    pub(in crate::gpu::evaluation) fn submit_work(
        &self,
        plan: Plan,
        inputs: &[&wgpu::Buffer],
    ) -> Result<WorkCounts, Error> {
        let recorded = self.executor().prepare(self.context.device(), plan)?;
        let mut encoder = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        let mut stats = recorded.encode_work(self.context.device(), &mut encoder, inputs)?;
        stats.include(recorded.preparation.clone());
        self.context
            .submit_with(self.queue, self.gate, encoder, self.cancelled, || {
                recorded.submitted()
            })?
            .ok_or(Error::Cancelled)?;
        stats.submitted();
        Ok(stats)
    }

    pub(in crate::gpu::evaluation) fn executor(&self) -> Executor {
        Executor::with_scope(self.context.programs.clone(), self.scope.clone())
            .with_working(self.working.clone())
    }
}

impl Context {
    pub(crate) fn run_sink<E, S>(
        &self,
        executor: &mut Executor,
        (queue, gate): (&wgpu::Queue, &crate::gpu::submission::SubmissionGate),
        plan: &Plan,
        (cancelled, progress): (&dyn Fn() -> bool, &dyn Fn()),
        sink: Sink<E, S>,
    ) -> Result<Option<WorkCounts>, Error>
    where
        E: FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
        S: FnMut(),
    {
        let scope = executor.materializations();
        let working = executor.working.clone();
        let result = executor.run_produced_sink(
            self.device(),
            (queue, gate),
            plan,
            cancelled,
            sink,
            |image, output, (pixels, available)| {
                let budget = Budget {
                    tile: [output.region.width.min(512), output.region.height.min(128)],
                    scratch_bytes: available,
                };
                let evaluation = self
                    .request(
                        image.clone(),
                        Region {
                            bounds: output.region,
                            grid: output.domain.grid,
                        },
                        budget,
                    )?
                    .with_materializations(scope.clone());
                let worker = Worker {
                    context: self,
                    queue,
                    gate,
                    scope: &scope,
                    working: &working,
                    budget,
                    cancelled,
                    progress,
                };
                let mut counts = WorkCounts::default();
                if let Some((region, host)) =
                    worker.host_image(image, output.region, &mut counts)?
                {
                    let values = host.as_host().expect("host image dependency");
                    // Produced arguments precede parent encoding. The parent's submission
                    // flushes these writes; only that parent publishes its GPU result.
                    for row in 0..output.region.height {
                        if cancelled() {
                            return Err(Error::Cancelled);
                        }
                        let from = region.index([output.region.x, output.region.y + row as i32]);
                        queue.write_buffer(
                            pixels,
                            u64::from(row) * u64::from(output.region.width) * 16,
                            bytemuck::cast_slice(
                                &values[from..from + output.region.width as usize],
                            ),
                        );
                    }
                    return Ok(counts);
                }
                Production {
                    worker,
                    layout: ImageLayout {
                        domain: output.domain,
                        region: output.region,
                        format: ImageFormat::Rgba32Float,
                        backing: ImageBacking::Buffer,
                    },
                    pixels,
                }
                .produce(&evaluation)
            },
        );
        match result {
            Err(Error::Cancelled) if cancelled() => Ok(None),
            result => result,
        }
    }
}
