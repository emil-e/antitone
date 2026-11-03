//! Worker execution capabilities, separate from the image definition and optional retention.
use super::*;

pub struct Production<'a> {
    pub context: &'a Context,
    pub queue: &'a wgpu::Queue,
    pub gate: &'a crate::gpu::SubmissionGate,
    pub scope: &'a Materializations,
    pub budget: Budget,
    pub layout: ImageLayout,
    pub pixels: &'a wgpu::Buffer,
    pub cancelled: &'a dyn Fn() -> bool,
    pub progress: &'a dyn Fn(),
}

impl Production<'_> {
    pub(crate) fn produce(&self, evaluation: &Evaluation) -> Result<Stats, Error> {
        let evaluation = evaluation.clone().with_materializations(self.scope.clone());
        let node = evaluation.job.image();
        let stored = super::super::materialized::Stored::find(
            Some(self.scope),
            &[],
            node,
            self.layout.region,
        )?
        .is_some();
        if !stored
            && let Some(producer) = node
                .operation
                .sample_consumer()
                .and_then(|consumer| consumer.production())
        {
            producer.produce(
                self,
                node.input
                    .as_ref()
                    .ok_or(Error::InvalidParameter("sampling consumer has no input"))?,
            )
        } else {
            self.evaluate(&evaluation)
        }
    }

    pub(super) fn evaluate(&self, evaluation: &Evaluation) -> Result<Stats, Error> {
        let mut executor = self.executor();
        let mut stats = Stats::default();
        for plan in evaluation.plans() {
            stats.include(self.deliver(&mut executor, plan?)?, 0);
        }
        Ok(stats)
    }

    pub(in crate::gpu::evaluation) fn run_sink(
        &self,
        executor: &mut Executor,
        plan: &Plan,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<Stats>, Error> {
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

    pub(in crate::gpu::evaluation) fn submit_work(
        &self,
        plan: Plan,
        inputs: &[&wgpu::Buffer],
    ) -> Result<Stats, Error> {
        let recorded = self.executor().prepare(self.context.device(), plan)?;
        let mut encoder = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        let mut stats = recorded.encode_work(self.context.device(), &mut encoder, inputs)?;
        stats.include(recorded.preparation.clone(), 0);
        stats.observe_device(self.context.device());
        self.context
            .submit_with(self.queue, self.gate, encoder, self.cancelled, || {
                recorded.submitted()
            })?
            .ok_or(Error::Cancelled)?;
        stats.submissions += 1;
        Ok(stats)
    }

    pub(in crate::gpu::evaluation) fn executor(&self) -> Executor {
        Executor::with_scope(self.context.programs.clone(), self.scope.clone())
    }

    pub(in crate::gpu::evaluation) fn deliver(
        &self,
        executor: &mut Executor,
        mut plan: Plan,
    ) -> Result<Stats, Error> {
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
                recorded.encode_result_stats(self.context.device(), &mut encoder, self.pixels)?;
            stats.include(recorded.preparation.clone(), 0);
            stats.observe_device(self.context.device());
            self.context
                .submit_with(self.queue, self.gate, encoder, self.cancelled, || {
                    recorded.submitted()
                })?
                .ok_or(Error::Cancelled)?;
            stats.submissions += 1;
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

impl Context {
    pub(crate) fn run_sink<E, S>(
        &self,
        executor: &mut Executor,
        (queue, gate): (&wgpu::Queue, &crate::gpu::SubmissionGate),
        plan: &Plan,
        (cancelled, progress): (&dyn Fn() -> bool, &dyn Fn()),
        sink: Sink<E, S>,
    ) -> Result<Option<Stats>, Error>
    where
        E: FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
        S: FnMut(),
    {
        let scope = executor.materializations();
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
                Production {
                    context: self,
                    queue,
                    gate,
                    scope: &scope,
                    budget,
                    layout: ImageLayout {
                        domain: output.domain,
                        region: output.region,
                        format: ImageFormat::Rgba32Float,
                        backing: ImageBacking::Buffer,
                    },
                    pixels,
                    cancelled,
                    progress,
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
