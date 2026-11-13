//! Execute stage-declared units through common image production and run-owned values.
use super::*;
use std::{
    future::Future,
    pin::pin,
    task::{Context as TaskContext, Poll, Wake},
    time::Duration,
};

mod assembly;
#[cfg(all(test, feature = "editor"))]
mod tests;

impl Production<'_> {
    pub(super) fn produce_units(&self, node: &value::Value) -> Result<WorkCounts, Error> {
        enum Unit {
            Ready(Image<ImageLayout>),
            Produce(WorkUnit),
        }
        let mut counts = WorkCounts::default();
        let mut covered = Vec::new();
        let mut copies = assembly::Copies::default();
        let mut regions = node.operation.work_units(self.layout.region, self.budget)?;
        let mut pending = Vec::new();
        while let Some(region) = pending.pop().or_else(|| regions.next()) {
            (self.progress)();
            if (self.cancelled)() {
                return Err(Error::Cancelled);
            }
            let unit = if let Some(image) = self.ready_unit(node, region)? {
                Unit::Ready(image)
            } else {
                let candidate = super::super::unit::plan(node, region, self.budget)
                    .and_then(|unit| unit.admit_for(self.budget, &self.context.device().limits()));
                let unit = match candidate {
                    Ok(unit) => unit,
                    Err(error @ Error::WorkingAllowance(_)) => {
                        let Some([first, second]) = super::super::unit::bisect(region) else {
                            return Err(error);
                        };
                        pending.extend([second, first]);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                validate_unit(node, &unit)?;
                Unit::Produce(unit)
            };
            let output = match &unit {
                Unit::Ready(image) => image.descriptor.region,
                Unit::Produce(unit) => unit.output,
            };
            let Some(overlap) = intersection(output, self.layout.region) else {
                continue;
            };
            if covered
                .iter()
                .any(|other| intersection(*other, overlap).is_some())
            {
                return Err(Error::InvalidParameter("stage work units overlap"));
            }
            covered.push(overlap);
            let image = match unit {
                Unit::Ready(image) => {
                    self.scope.used(&image.image);
                    self.working.hold(image.image.clone());
                    image.image
                }
                Unit::Produce(unit) => {
                    self.unit_image_before(node, unit, &mut counts, || copies.flush(self))?
                }
            };
            copies.copy(self, image, output, overlap)?;
        }
        if covered
            .iter()
            .map(|region| u64::from(region.width) * u64::from(region.height))
            .sum::<u64>()
            != u64::from(self.layout.region.width) * u64::from(self.layout.region.height)
        {
            return Err(Error::InvalidParameter(
                "stage work units do not cover the demand",
            ));
        }
        copies.flush(self)?;
        counts.include(copies.counts);
        Ok(counts)
    }
}

impl Worker<'_> {
    fn ready_unit(
        &self,
        node: &value::Value,
        region: CellRegion,
    ) -> Result<Option<Image<ImageLayout>>, Error> {
        let output = node.output(region)?;
        let Some(identity) = output.identity() else {
            return Ok(None);
        };
        let image = self
            .scope
            .ready::<ImageLayout>(identity)
            .into_iter()
            .find(|image| {
                let layout = image.descriptor.as_ref();
                layout.domain == node.domain()
                    && layout.region == region
                    && layout.format == ImageFormat::Rgba32Float
                    && matches!(layout.backing, ImageBacking::Host | ImageBacking::Buffer)
            });
        if let Some(image) = &image {
            super::super::materialized::validate(node.domain(), *image.descriptor, &image.image)?;
        }
        Ok(image)
    }

    pub(super) fn host_image(
        &self,
        node: &value::Value,
        region: CellRegion,
        counts: &mut WorkCounts,
    ) -> Result<Option<(CellRegion, Arc<ImageValue>)>, Error> {
        if let Some(identity) = node.output(region)?.identity()
            && let Some(image) =
                self.scope
                    .ready::<ImageLayout>(identity)
                    .into_iter()
                    .find(|image| {
                        image.descriptor.backing == ImageBacking::Host
                            && image.descriptor.domain == node.domain()
                            && image.descriptor.region.contains_region(region)
                    })
        {
            super::super::materialized::validate(node.domain(), *image.descriptor, &image.image)?;
            self.scope.used(&image.image);
            self.working.hold(image.image.clone());
            return Ok(Some((image.descriptor.region, image.image)));
        }
        if Stored::find(Some(self.scope), &[], node, region)?.is_some() {
            return Ok(None);
        }
        let probe = CellRegion::new(
            [region.x, region.y],
            [
                region.width.min(self.budget.tile[0]),
                region.height.min(self.budget.tile[1]),
            ],
        );
        let unit = match matching_unit(node, probe, self.budget) {
            Err(Error::WorkingAllowance(_)) | Ok(None) => return Ok(None),
            Err(error) => return Err(error),
            Ok(Some(unit)) => unit,
        };
        if !matches!(&unit.work, UnitWork::Cpu(_)) || !unit.output.contains_region(region) {
            return Ok(None);
        }
        let output = unit.output;
        Ok(Some((output, self.unit_image(node, unit, counts)?)))
    }

    pub(super) fn materialize_image(
        &self,
        evaluation: &Evaluation,
        layout: ImageLayout,
        counts: &mut WorkCounts,
    ) -> Result<Arc<ImageValue>, Error> {
        let node = evaluation.job.image();
        if let Some(identity) = node.output(layout.region)?.identity()
            && let Some(image) = self
                .scope
                .ready::<ImageLayout>(identity)
                .into_iter()
                .find(|image| *image.descriptor == layout)
        {
            super::super::materialized::validate(node.domain(), layout, &image.image)?;
            self.scope.used(&image.image);
            return Ok(image.image);
        }
        if Stored::find(Some(self.scope), &[], node, layout.region)?.is_none()
            && !super::super::unit::can_inline(node, layout.region, self.budget)?
            && matches!(
                super::super::unit::lowering(node, layout.region, self.budget)?,
                super::super::unit::Lowering::Deferred
            )
            && let Some(unit) = matching_unit(node, layout.region, self.budget)?
            && unit.output == layout.region
            && matches!(&unit.work, UnitWork::Gpu(_))
        {
            match super::super::unit::plan(node, layout.region, self.budget)?
                .admit_for(self.budget, &self.context.device().limits())
            {
                Ok(unit) => return self.unit_image(node, unit, counts),
                Err(Error::WorkingAllowance(_)) => {}
                Err(error) => return Err(error),
            }
        }
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::evaluation_buffer_created(layout.bytes());
        let pixels = self
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("materialized image"),
                size: layout.bytes(),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        counts.buffers_allocated(1);
        let worker = Production {
            worker: *self,
            layout,
            pixels: &pixels,
        };
        counts.include(worker.produce(evaluation)?);
        Ok(ImageValue::new(pixels))
    }

    fn unit_image(
        &self,
        node: &value::Value,
        unit: WorkUnit,
        counts: &mut WorkCounts,
    ) -> Result<Arc<ImageValue>, Error> {
        self.unit_image_before(node, unit, counts, || Ok(()))
    }

    fn unit_image_before(
        &self,
        node: &value::Value,
        unit: WorkUnit,
        counts: &mut WorkCounts,
        mut before_work: impl FnMut() -> Result<(), Error>,
    ) -> Result<Arc<ImageValue>, Error> {
        let image = if let Some(identity) = node.output(unit.output)?.identity() {
            let layout = ImageLayout {
                domain: unit.domain,
                region: unit.output,
                format: ImageFormat::Rgba32Float,
                backing: if matches!(&unit.work, UnitWork::Cpu(_)) {
                    ImageBacking::Host
                } else {
                    ImageBacking::Buffer
                },
            };
            let (pending, producer) =
                self.scope
                    .claim_unit(identity.clone(), Arc::new(layout), self.budget);
            if let Some(producer) = producer {
                let assembly_error = before_work().err();
                // The drain belongs to this assembly, but joined consumers still own production.
                if let Some(error) = &assembly_error
                    && producer.cancel_when_unshared()
                {
                    return Err(error.clone());
                }
                let cancelled = || (self.cancelled)() && producer.cancel_when_unshared();
                let worker = Worker {
                    cancelled: &cancelled,
                    ..*self
                };
                let computed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.compute_unit(node, unit)
                }))
                .unwrap_or_else(|_| Err(Error::Gpu("calculation unit worker panicked".into())));
                let result = computed.map(|(image, physical)| {
                    counts.include(physical);
                    image
                });
                producer.complete(result);
                if let Some(error) = assembly_error {
                    return Err(error);
                }
            }
            let image =
                wait_image_before(pending, self.cancelled, self.progress, &mut before_work)?.image;
            super::super::materialized::validate(node.domain(), layout, &image)?;
            image
        } else {
            before_work()?;
            let (image, physical) = self.compute_unit(node, unit)?;
            counts.include(physical);
            image
        };
        self.working.hold(image.clone());
        Ok(image)
    }

    pub(super) fn compute_unit(
        &self,
        node: &value::Value,
        unit: WorkUnit,
    ) -> Result<(Arc<ImageValue>, WorkCounts), Error> {
        if (self.cancelled)() {
            return Err(Error::Cancelled);
        }
        validate_unit(node, &unit)?;
        let mut counts = WorkCounts::default();
        if let UnitWork::Cpu(work) = &unit.work {
            let pixels = self.cpu_pixels(&unit.inputs, work, unit.output, &mut counts)?;
            return Ok((ImageValue::host(pixels), counts));
        }
        let bytes = rgba_bytes(unit.output)?;
        let limits = self.context.device().limits();
        if bytes > self.budget.scratch_bytes
            || bytes > limits.max_buffer_size
            || bytes > limits.max_storage_buffer_binding_size
        {
            return Err(Error::WorkingAllowance(
                "work-unit result exceeds the working allowance",
            ));
        }
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::evaluation_buffer_created(bytes);
        let output = self
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("declared calculation unit"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        counts.buffers_allocated(1);
        if matches!(&unit.work, UnitWork::Gpu(_)) {
            let evaluation = self.query(node.clone(), unit.output, self.budget.scratch_bytes)?;
            let plan = evaluation.job.probe(unit.output)?;
            if !plan.requires_production(node)?
                && plan.peak_scratch_bytes <= self.budget.scratch_bytes
                && Executor::admit(&self.context.device().limits(), &plan).is_ok()
            {
                counts.include(
                    Production {
                        worker: *self,
                        layout: ImageLayout {
                            domain: unit.domain,
                            region: unit.output,
                            format: ImageFormat::Rgba32Float,
                            backing: ImageBacking::Buffer,
                        },
                        pixels: &output,
                    }
                    .deliver(&mut self.executor(), plan)?,
                );
                return Ok((ImageValue::new(output), counts));
            }
        }
        match unit.work {
            UnitWork::Gpu(work) => {
                counts.include(self.gpu_unit(unit.inputs, unit.output, work, &output)?)
            }
            UnitWork::Source(source) => {
                let values = source.read(unit.output)?;
                if values.len() != cells(unit.output)?
                    || values.iter().flatten().any(|value| !value.is_finite())
                {
                    return Err(Error::InvalidParameter(
                        "source result differs from its unit layout",
                    ));
                }
                if (self.cancelled)() {
                    return Err(Error::Cancelled);
                }
                self.queue
                    .write_buffer(&output, 0, bytemuck::cast_slice(&values));
                let encoder = self
                    .context
                    .device()
                    .create_command_encoder(&Default::default());
                self.context
                    .submit(self.queue, self.gate, encoder, self.cancelled)?
                    .ok_or(Error::Cancelled)?;
                counts.submitted();
            }
            UnitWork::Cpu(_) => unreachable!(),
        }
        Ok((ImageValue::new(output), counts))
    }

    pub(super) fn gpu_unit(
        &self,
        inputs: Vec<value::Value>,
        region: CellRegion,
        work: GpuUnit,
        output: &wgpu::Buffer,
    ) -> Result<WorkCounts, Error> {
        let mut counts = WorkCounts::default();
        let mut run =
            super::gpu::GpuExecution::new(self, inputs, region, work, output, &mut counts)?;
        while let Some(phase) = run.phases.next() {
            run.execute(self, phase?, 0, &mut Vec::new(), &mut counts)?;
        }
        run.finish()?;
        Ok(counts)
    }

    pub(super) fn cpu_pixels(
        &self,
        upstreams: &[value::Value],
        work: &CpuUnit,
        output: CellRegion,
        counts: &mut WorkCounts,
    ) -> Result<Vec<[f32; 4]>, Error> {
        let mut inputs = Vec::with_capacity(work.reads.len());
        for read in &work.reads {
            (self.progress)();
            let upstream = upstream(upstreams, *read)?;
            let mut values = vec![[0.; 4]; cells(read.region)?];
            let query = self.query(upstream.clone(), read.region, self.budget.scratch_bytes)?;
            let stats = self
                .context
                .read_in(
                    (self.queue, self.gate),
                    &query,
                    (self.cancelled, self.progress),
                    self.working.clone(),
                    |block, pixels| {
                        copy_host(block, pixels, read.region, &mut values);
                        Ok(())
                    },
                )?
                .ok_or(Error::Cancelled)?;
            counts.include(stats);
            inputs.push(CpuImage {
                domain: upstream.domain(),
                region: read.region,
                pixels: values,
            });
        }
        let values = work
            .kernel
            .run(&inputs, output, self.cancelled)?
            .ok_or(Error::Cancelled)?;
        if values.len() != cells(output)? {
            return Err(Error::InvalidParameter(
                "CPU result differs from its declared layout",
            ));
        }
        if (self.cancelled)() {
            return Err(Error::Cancelled);
        }
        Ok(values)
    }

    pub(super) fn query(
        &self,
        node: value::Value,
        region: CellRegion,
        bytes: u64,
    ) -> Result<Evaluation, Error> {
        Ok(self
            .context
            .request(
                node.clone(),
                Region {
                    bounds: region,
                    grid: node.domain().grid,
                },
                // Input transfer chunks follow this read, not the consumer's output tile.
                Budget {
                    tile: [region.width.min(512), region.height.min(128)],
                    scratch_bytes: bytes,
                },
            )?
            .with_materializations(self.scope.clone()))
    }
}

pub(super) fn matching_unit(
    node: &value::Value,
    region: CellRegion,
    budget: Budget,
) -> Result<Option<WorkUnit>, Error> {
    let mut regions = node.operation.work_units(region, budget)?;
    let Some(first) = regions.next() else {
        return Ok(None);
    };
    if regions.next().is_some() {
        return Ok(None);
    }
    let candidate =
        super::super::unit::plan(node, first, budget).and_then(|unit| unit.admit(budget));
    let unit = match candidate {
        Ok(unit) => unit,
        Err(Error::WorkingAllowance(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    validate_unit(node, &unit)?;
    Ok(unit.output.contains_region(region).then_some(unit))
}

fn validate_unit(node: &value::Value, unit: &WorkUnit) -> Result<(), Error> {
    unit.output.validate()?;
    if unit.domain != node.domain() || !node.domain().contains_region(unit.output) {
        return Err(Error::InvalidParameter(
            "work unit differs from its image domain",
        ));
    }
    Ok(())
}

struct WaitingThread(std::thread::Thread);
impl Wake for WaitingThread {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
pub(super) fn wait_image<D>(
    pending: ImageFuture<D>,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(),
) -> Result<Image<D>, Error> {
    wait_image_before(pending, cancelled, progress, || Ok(()))
}
fn wait_image_before<D>(
    pending: ImageFuture<D>,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(),
    mut before_wait: impl FnMut() -> Result<(), Error>,
) -> Result<Image<D>, Error> {
    let waker = Arc::new(WaitingThread(std::thread::current())).into();
    let mut cx = TaskContext::from_waker(&waker);
    let mut pending = pin!(pending);
    loop {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        match pending.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result.map_err(|error| (*error).clone()),
            Poll::Pending => {
                before_wait()?;
                progress();
                // Owner cancellation need not wake the shared producer.
                std::thread::park_timeout(Duration::from_millis(8));
            }
        }
    }
}

fn upstream(inputs: &[value::Value], read: Read) -> Result<&value::Value, Error> {
    read.region.validate()?;
    let input = inputs
        .get(read.input)
        .ok_or(Error::InvalidParameter("read references an unknown input"))?;
    if !input.domain().contains_region(read.region) {
        return Err(Error::InvalidParameter("unit read exceeds upstream image"));
    }
    Ok(input)
}
fn cells(region: CellRegion) -> Result<usize, Error> {
    region.validate()?;
    usize::try_from(u64::from(region.width) * u64::from(region.height))
        .map_err(|_| Error::InvalidParameter("logical input is too large"))
}
fn rgba_bytes(region: CellRegion) -> Result<u64, Error> {
    u64::try_from(cells(region)?)
        .ok()
        .and_then(|cells| cells.checked_mul(16))
        .ok_or(Error::InvalidParameter("logical input byte size overflows"))
}
fn intersection(a: CellRegion, b: CellRegion) -> Option<CellRegion> {
    let origin = [a.x.max(b.x), a.y.max(b.y)];
    let end = [a.right().min(b.right()), a.bottom().min(b.bottom())];
    (end[0] > origin[0] && end[1] > origin[1]).then(|| {
        CellRegion::new(
            origin,
            [(end[0] - origin[0]) as u32, (end[1] - origin[1]) as u32],
        )
    })
}
fn copy_host(block: CellRegion, values: &[[f32; 4]], region: CellRegion, output: &mut [[f32; 4]]) {
    for row in 0..block.height {
        let from = row as usize * block.width as usize;
        let to = region.index([block.x, block.y + row as i32]);
        output[to..to + block.width as usize]
            .copy_from_slice(&values[from..from + block.width as usize]);
    }
}
