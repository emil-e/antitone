//! Persistent GPU state and phase-local ordinary image ownership.
use super::*;
use crate::gpu::allocation::DeviceAllocationExt;

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(super) struct GpuExecution {
    inputs: Vec<value::Value>,
    states: Vec<wgpu::Buffer>,
    output: wgpu::Buffer,
    pub(super) phases: Box<dyn Iterator<Item = Result<GpuPhase, Error>> + Send>,
    has_output: bool,
}
impl GpuExecution {
    pub(super) fn new(
        worker: &Worker<'_>,
        inputs: Vec<value::Value>,
        _region: CellRegion,
        work: GpuUnit,
        output: &wgpu::Buffer,
        counts: &mut WorkCounts,
    ) -> Result<Self, Error> {
        let limits = worker.context.device().limits();
        worker.working.admit_gpu(worker.context.device())?;
        let states = work
            .state
            .into_iter()
            .map(|size| {
                if size == 0
                    || size % 4 != 0
                    || size > limits.max_buffer_size
                    || size > limits.max_storage_buffer_binding_size
                {
                    return Err(Error::InvalidParameter(
                        "unit state exceeds GPU storage limits",
                    ));
                }
                #[cfg(all(test, feature = "editor"))]
                crate::evaluation::test_support::evaluation_buffer_created(size);
                worker
                    .context
                    .device()
                    .try_create_buffer(&wgpu::BufferDescriptor {
                        label: Some("calculation unit state"),
                        size,
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_SRC
                            | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        counts.buffers_allocated(states.len());
        Ok(Self {
            inputs,
            states,
            output: output.clone(),
            phases: work.phases,
            has_output: false,
        })
    }

    pub(super) fn execute(
        &mut self,
        worker: &Worker<'_>,
        phase: GpuPhase,
        reads_owner: &mut Vec<Arc<ImageValue>>,
        counts: &mut WorkCounts,
    ) -> Result<(), Error> {
        (worker.progress)();
        if (worker.cancelled)() {
            return Err(Error::Cancelled);
        }
        self.has_output |= phase.inputs.contains(&UnitValue::Output);
        let limits = worker.context.device().limits();
        let mut reads = Vec::with_capacity(phase.reads.len());
        let mut captures = Vec::<(value::Value, CellRegion, Arc<ImageValue>)>::new();
        for read in phase.reads {
            read.region.validate()?;
            let upstream = self
                .inputs
                .get(read.input)
                .filter(|input| input.domain().contains_region(read.region))
                .ok_or(Error::InvalidParameter("unit read exceeds upstream image"))?;
            let layout = ImageLayout {
                domain: upstream.domain(),
                region: read.region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            let bytes = layout.bytes();
            if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
                return Err(Error::InvalidParameter(
                    "phase read exceeds GPU storage limits",
                ));
            }
            if let Some((_, _, image)) = captures
                .iter()
                .find(|(input, region, _)| input.same_capture(upstream) && *region == read.region)
            {
                reads.push(image.clone());
                continue;
            }
            let output = upstream.output(read.region)?;
            let ready = output.identity().and_then(|identity| {
                worker
                    .scope
                    .ready::<ImageLayout>(identity)
                    .into_iter()
                    .find(|image| *image.descriptor == layout)
            });
            if let Some(image) = ready.as_ref().filter(|image| {
                reads_owner
                    .iter()
                    .any(|owner| Arc::ptr_eq(owner, &image.image))
            }) {
                super::super::materialized::validate(upstream.domain(), layout, &image.image)?;
                captures.push((upstream.clone(), read.region, image.image.clone()));
                reads.push(image.image.clone());
                continue;
            }
            let input = Worker { ..*worker };
            let query = input.query(upstream.clone(), read.region)?;
            let image = input.materialize_image(&query, layout, counts)?;
            // Fully tiled sources publish their acquisition units; this phase read
            // can instead be a transient assembly across several of them.
            if upstream
                .source()
                .is_none_or(|source| source.acquisition_tile().is_none())
                && let Some(identity) = output.identity()
            {
                worker.scope.submitted_image(&Image {
                    identity: identity.clone(),
                    descriptor: Arc::new(layout),
                    image: image.clone(),
                });
            }
            // Discovery remains in Materializations. These handles only keep this round's
            // shared reads alive until the last scheduled consumer has submitted.
            if !reads_owner.iter().any(|owner| Arc::ptr_eq(owner, &image)) {
                reads_owner.push(image.clone());
            }
            captures.push((upstream.clone(), read.region, image.clone()));
            reads.push(image);
        }
        let mut fragment = Fragment::with_programs(Some(worker.context.programs.clone()));
        let mut buffers = Vec::new();
        let mut slots = Vec::new();
        for (slot, value) in phase.inputs.into_iter().enumerate() {
            let (buffer, writable) = match value {
                UnitValue::Read(index) => (
                    reads
                        .get(index)
                        .and_then(|image| image.as_buffer())
                        .ok_or(Error::InvalidParameter("unknown GPU phase read"))?,
                    false,
                ),
                UnitValue::State(index) => (
                    self.states
                        .get(index)
                        .ok_or(Error::InvalidParameter("unknown unit state"))?,
                    true,
                ),
                UnitValue::Output => (&self.output, true),
            };
            slots.push(fragment.resource(Storage::Input {
                slot,
                bytes: buffer.size(),
                writable,
            }));
            buffers.push(buffer);
        }
        let result = phase.kernel.lower(&mut fragment, &slots)?;
        counts.include(worker.submit_work(Plan::from_work(fragment, result)?, &buffers, &reads)?);
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<(), Error> {
        if !self.has_output {
            return Err(Error::InvalidParameter("GPU unit has no output phase"));
        }
        Ok(())
    }
}
