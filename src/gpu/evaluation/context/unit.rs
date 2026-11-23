//! Execute stage-declared units through common image production and run-owned values.
use super::*;
use crate::gpu::allocation::DeviceAllocationExt;
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
    pub(super) fn copy_ready(&self, node: &value::Value) -> Result<Option<WorkCounts>, Error> {
        if self.layout.format != ImageFormat::Rgba32Float
            || self.layout.backing != ImageBacking::Buffer
            || !self.pixels.usage().contains(wgpu::BufferUsages::COPY_DST)
        {
            return Ok(None);
        }
        if self.pixels.size() < self.layout.bytes() {
            return Err(Error::InvalidParameter(
                "covered image exceeds its destination",
            ));
        }
        let Some(cover) = copy_cover(self.scope, node, self.layout.region)? else {
            return Ok(None);
        };
        if cover.regions.iter().any(|part| {
            part.image.as_buffer() == Some(self.pixels) && part.layout.region != self.layout.region
        }) {
            return Ok(None);
        }
        let mut copies = assembly::Copies::default();
        for part in cover.regions {
            (self.progress)();
            if (self.cancelled)() {
                return Err(Error::Cancelled);
            }
            self.scope.used(&part.image);
            if part.image.as_buffer() != Some(self.pixels) {
                copies.copy(self, part.image, part.layout.region, part.region)?;
            }
        }
        copies.flush(self)?;
        Ok(Some(copies.counts))
    }

    pub(super) fn produce_units(
        &self,
        node: &value::Value,
        tiling: Tiling,
    ) -> Result<WorkCounts, Error> {
        enum Unit {
            Ready(Image<ImageLayout>),
            Produce(WorkUnit),
        }
        let mut counts = WorkCounts::default();
        let mut covered = Vec::new();
        let mut copies = assembly::Copies::default();
        for region in node.operation.work_units(self.layout.region, tiling)? {
            (self.progress)();
            if (self.cancelled)() {
                return Err(Error::Cancelled);
            }
            let unit = if let Some(image) = self.ready_unit(node, region)? {
                Unit::Ready(image)
            } else {
                let unit = super::super::unit::plan_with_limits(
                    node,
                    region,
                    tiling,
                    Some(&self.context.device().limits()),
                )?
                .admit_for(&self.context.device().limits())?;
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
        let tiling = Tiling { tile: [512, 128] };
        let probe = CellRegion::new(
            [region.x, region.y],
            [
                region.width.min(tiling.tile[0]),
                region.height.min(tiling.tile[1]),
            ],
        );
        let unit = match matching_unit(node, probe, tiling, None) {
            Err(Error::EvaluationLimit(_)) | Ok(None) => return Ok(None),
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
        let tiling = evaluation.job.tiling();
        let limits = self.context.device().limits();
        if layout.bytes() > limits.max_buffer_size
            || layout.bytes() > limits.max_storage_buffer_binding_size
        {
            return Err(Error::EvaluationLimit(
                "materialized image exceeds GPU storage limits",
            ));
        }
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
            && !super::super::unit::can_inline(node, layout.region, tiling, Some(&limits))?
            && matches!(
                super::super::unit::lowering(node, layout.region, tiling, Some(&limits))?,
                super::super::unit::Lowering::Deferred
            )
            && let Some(unit) = matching_unit(node, layout.region, tiling, Some(&limits))?
            && unit.output == layout.region
            && matches!(&unit.work, UnitWork::Gpu(_))
        {
            match super::super::unit::plan_with_limits(node, layout.region, tiling, Some(&limits))?
                .admit_for(&self.context.device().limits())
            {
                Ok(unit) => return self.unit_image(node, unit, counts),
                Err(Error::EvaluationLimit(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.working.admit_gpu(self.context.device())?;
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::evaluation_buffer_created(layout.bytes());
        let pixels = self
            .context
            .device()
            .try_create_buffer(&wgpu::BufferDescriptor {
                label: Some("materialized image"),
                size: layout.bytes(),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })?;
        counts.buffers_allocated(1);
        let worker = Production {
            worker: *self,
            layout,
            pixels: &pixels,
        };
        counts.include(worker.produce(evaluation)?);
        Ok(ImageValue::new(pixels))
    }

    pub(super) fn direct_gpu_input(
        &self,
        node: &value::Value,
        layout: ImageLayout,
        counts: &mut WorkCounts,
    ) -> Result<Option<Arc<ImageValue>>, Error> {
        let limits = self.context.device().limits();
        let budget = Tiling::for_input(layout.region);
        let Some(identity) = node.output(layout.region)?.identity().cloned() else {
            return Ok(None);
        };
        let ready = self.scope.ready::<ImageLayout>(&identity);
        if ready.iter().any(|image| {
            image.descriptor.backing == ImageBacking::Host
                && image.descriptor.domain == layout.domain
                && image.descriptor.region.contains_region(layout.region)
        }) {
            return Ok(None);
        }
        if let Some(image) = ready.into_iter().find(|image| {
            *image.descriptor == layout
                && image
                    .as_buffer()
                    .is_some_and(|buffer| buffer.usage().contains(wgpu::BufferUsages::COPY_SRC))
        }) {
            super::super::materialized::validate(node.domain(), layout, &image.image)?;
            return Ok(Some(image.image));
        }

        let Some(unit) = matching_unit(node, layout.region, budget, Some(&limits))? else {
            return Ok(None);
        };
        if unit.output != layout.region || !matches!(&unit.work, UnitWork::Gpu(_)) {
            return Ok(None);
        }
        // Keep existing assembly for reusable pixels that cannot be bound as-is.
        if Stored::find(Some(self.scope), &[], node, layout.region)?.is_some()
            || copy_cover(self.scope, node, layout.region)?.is_some()
        {
            return Ok(None);
        }

        let unit = match unit.admit_for(&limits) {
            Ok(unit) => unit,
            Err(Error::EvaluationLimit(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let image = self.unit_image(node, unit, counts)?;
        // The claim can also join an existing allocation. If it cannot serve as a
        // copy source, leave the original delivery path in charge of the binding.
        Ok(image
            .as_buffer()
            .is_some_and(|buffer| buffer.usage().contains(wgpu::BufferUsages::COPY_SRC))
            .then_some(image))
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
            let (pending, producer) = self.scope.claim_unit(identity.clone(), Arc::new(layout));
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
        if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
            return Err(Error::EvaluationLimit(
                "work-unit result exceeds GPU storage limits",
            ));
        }
        self.working.admit_gpu(self.context.device())?;
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::evaluation_buffer_created(bytes);
        let output = self
            .context
            .device()
            .try_create_buffer(&wgpu::BufferDescriptor {
                label: Some("declared calculation unit"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })?;
        counts.buffers_allocated(1);
        if matches!(&unit.work, UnitWork::Gpu(_)) {
            let evaluation = self.query(node.clone(), unit.output)?;
            let plan = evaluation.job.probe(unit.output)?;
            if !plan.requires_production(node)?
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
                    .submit(self.queue, encoder, self.cancelled)?
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
            run.execute(self, phase?, &mut Vec::new(), &mut counts)?;
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
            let query = self.query(upstream.clone(), read.region)?;
            let stats = self
                .context
                .read_in(
                    self.queue,
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
    ) -> Result<Evaluation, Error> {
        Ok(self
            .context
            .request(
                node.clone(),
                Region {
                    bounds: region,
                    grid: node.domain().grid,
                },
                Tiling::for_input(region),
            )?
            .with_materializations(self.scope.clone()))
    }
}

fn copy_cover(
    scope: &Materializations,
    node: &value::Value,
    region: CellRegion,
) -> Result<Option<super::super::materialized::CopyCover>, Error> {
    let mut remaining = value::MAX_UNIT_VALUES;
    super::super::materialized::CopyCover::find(
        scope,
        node,
        region,
        value::MAX_UNIT_VALUES,
        &mut remaining,
    )
}

pub(super) fn matching_unit(
    node: &value::Value,
    region: CellRegion,
    budget: Tiling,
    limits: Option<&wgpu::Limits>,
) -> Result<Option<WorkUnit>, Error> {
    let mut regions = node.operation.work_units(region, budget)?;
    let Some(first) = regions.next() else {
        return Ok(None);
    };
    if regions.next().is_some() {
        return Ok(None);
    }
    let candidate = super::super::unit::plan_with_limits(node, first, budget, limits);
    let unit = match candidate {
        Ok(unit) => unit,
        Err(Error::EvaluationLimit(_)) => return Ok(None),
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
