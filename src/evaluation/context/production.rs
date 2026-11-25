//! Worker execution capabilities, separate from the image definition and optional retention.
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone, Copy)]
pub struct Worker<'a> {
    pub context: &'a Context,
    pub queue: &'a crate::gpu::submission::Queue,
    pub scope: &'a Materializations,
    pub working: &'a RunWorkingSet,
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
    budget: Tiling,
    limits: wgpu::Limits,
    (cancelled, progress): (&'a dyn Fn() -> bool, &'a dyn Fn()),
) -> Result<impl Iterator<Item = Result<Plan, Error>> + 'a, Error> {
    Ok(tiled_units(region, budget.tile)?.map(move |region| {
        progress();
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let plan = evaluation.job.probe(region)?;
        if plan.requires_production(evaluation.job.image())? {
            super::super::unit::plan_with_limits(
                evaluation.job.image(),
                region,
                budget,
                Some(&limits),
            )?
            .admit_for(&limits)?;
        }
        Executor::admit(&limits, &plan)?;
        Ok(plan)
    }))
}

impl Production<'_> {
    pub(crate) fn produce(&self, evaluation: &Evaluation) -> Result<WorkCounts, Error> {
        let evaluation = evaluation.clone().with_materializations(self.scope.clone());
        let node = evaluation.job.image();
        let tiling = evaluation.job.tiling();
        if let Some(counts) = self.copy_ready(node)? {
            return Ok(counts);
        }
        if node
            .source()
            .is_some_and(|source| source.acquisition_tile().is_some())
        {
            return self.produce_units(node, tiling);
        }
        let stored = super::super::materialized::Stored::find(
            Some(self.scope),
            &[],
            node,
            self.layout.region,
        )?
        .is_some();
        if !stored
            && super::super::unit::can_inline(
                node,
                self.layout.region,
                tiling,
                Some(&self.context.device().limits()),
            )?
        {
            let plan = evaluation.job.probe(self.layout.region)?;
            let deferred = plan.requires_production(node)?
                || Executor::admit(&self.context.device().limits(), &plan).is_err();
            return if deferred {
                self.produce_units(node, tiling)
            } else {
                self.deliver(&mut self.executor(), plan)
            };
        }
        if !stored
            && matches!(
                super::super::unit::lowering(
                    node,
                    self.layout.region,
                    tiling,
                    Some(&self.context.device().limits())
                )?,
                super::super::unit::Lowering::Deferred
            )
        {
            return self.produce_units(node, tiling);
        }
        self.evaluate(&evaluation)
    }

    pub(super) fn evaluate(&self, evaluation: &Evaluation) -> Result<WorkCounts, Error> {
        let mut executor = self.executor();
        let mut stats = WorkCounts::default();
        for plan in admitted_plans(
            evaluation.clone(),
            self.layout.region,
            evaluation.job.tiling(),
            self.context.device().limits(),
            (self.cancelled, self.progress),
        )? {
            stats.include(self.deliver(&mut executor, plan?)?);
        }
        Ok(stats)
    }

    pub(in crate::evaluation) fn run_sink(
        &self,
        executor: &mut Executor,
        plan: &Plan,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
        self.context.run_sink(
            executor,
            self.queue,
            plan,
            (self.cancelled, self.progress),
            (
                Sink {
                    encode: sink,
                    submitted: || {},
                },
                Some(self.pixels),
            ),
        )
    }

    pub(in crate::evaluation) fn deliver(
        &self,
        executor: &mut Executor,
        plan: Plan,
    ) -> Result<WorkCounts, Error> {
        (self.progress)();
        if (self.cancelled)() {
            return Err(Error::Cancelled);
        }
        let resident = plan.recording_passes() <= Plan::MAX_RECORDING_PASSES
            && plan.region == self.layout.region
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
            let encoded =
                recorded.encode_result_owned(self.context.device(), &mut encoder, self.pixels)?;
            let mut stats = encoded.counts;
            stats.include(recorded.preparation.clone());
            let resources = encoded.resources;
            if (self.cancelled)() {
                return Err(Error::Cancelled);
            }
            self.working.submit_gpu(
                self.context.device(),
                self.queue,
                encoder,
                resources,
                || recorded.submitted(),
            )?;
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
    pub(in crate::evaluation) fn submit_work(
        &self,
        plan: Plan,
        inputs: &[&wgpu::Buffer],
        images: &[Arc<ImageValue>],
    ) -> Result<WorkCounts, Error> {
        let recorded = self.executor().prepare(self.context.device(), plan)?;
        let mut encoder = self
            .context
            .device()
            .create_command_encoder(&Default::default());
        let mut encoded =
            recorded.encode_work_owned(self.context.device(), &mut encoder, inputs)?;
        for image in images {
            encoded.resources.image(image);
        }
        let mut stats = encoded.counts;
        stats.include(recorded.preparation.clone());
        if (self.cancelled)() {
            return Err(Error::Cancelled);
        }
        self.working.submit_gpu(
            self.context.device(),
            self.queue,
            encoder,
            encoded.resources,
            || recorded.submitted(),
        )?;
        stats.submitted();
        Ok(stats)
    }

    pub(in crate::evaluation) fn executor(&self) -> Executor {
        Executor::with_scope(self.context.programs.clone(), self.scope.clone())
            .with_working(self.working.clone())
    }

    fn produce_input_into(
        &self,
        image: &value::Value,
        output: PlannedImage,
        pixels: &wgpu::Buffer,
    ) -> Result<WorkCounts, Error> {
        let evaluation = self
            .context
            .request(
                image.clone(),
                Region {
                    bounds: output.region,
                    grid: output.domain.grid,
                },
                Tiling::for_input(output.region),
            )?
            .with_materializations(self.scope.clone());
        let mut counts = WorkCounts::default();
        if let Some((region, host)) = self.host_image(image, output.region, &mut counts)? {
            let values = host.as_host().expect("host image dependency");
            // Only the parent's submission flushes these writes and publishes the GPU result.
            for row in 0..output.region.height {
                if (self.cancelled)() {
                    return Err(Error::Cancelled);
                }
                let from = region.index([output.region.x, output.region.y + row as i32]);
                self.queue.write_buffer(
                    pixels,
                    u64::from(row) * u64::from(output.region.width) * 16,
                    bytemuck::cast_slice(&values[from..from + output.region.width as usize]),
                );
            }
            return Ok(counts);
        }
        Production {
            worker: *self,
            layout: ImageLayout {
                domain: output.domain,
                region: output.region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            },
            pixels,
        }
        .produce(&evaluation)
    }
}

impl Context {
    pub(crate) fn run_sink<E, S>(
        &self,
        executor: &mut Executor,
        queue: &crate::gpu::submission::Queue,
        plan: &Plan,
        (cancelled, progress): (&dyn Fn() -> bool, &dyn Fn()),
        sink: (Sink<E, S>, Option<&wgpu::Buffer>),
    ) -> Result<Option<WorkCounts>, Error>
    where
        E: FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
        S: FnMut(),
    {
        let scope = executor.materializations();
        let working = executor.working.clone();
        let worker = Worker {
            context: self,
            queue,
            scope: &scope,
            working: &working,
            cancelled,
            progress,
        };
        let result = executor.run_produced_sink(
            self.device(),
            queue,
            plan,
            cancelled,
            sink,
            (
                |image, layout, counts| worker.direct_gpu_input(image, layout, counts),
                |image, output, pixels| worker.produce_input_into(image, output, pixels),
            ),
        );
        match result {
            Err(Error::Cancelled) if cancelled() => Ok(None),
            result => result,
        }
    }
}
