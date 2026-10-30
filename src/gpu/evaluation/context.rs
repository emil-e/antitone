//! Device-scoped evaluation policy; optional retention is separate from required work budgets.

use super::*;
use crate::geometry::SourceRegion;

mod readback;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    /// Optional generated/intermediate-buffer payload bytes across prepared work and image scopes.
    /// Zero keeps only required active inputs and working storage.
    pub generated_bytes: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            generated_bytes: 256 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub bounds: SourceRegion,
    pub grid: Grid,
}

#[derive(Clone)]
pub struct Context {
    device: wgpu::Device,
    pub(crate) programs: Arc<Programs>,
    retention: cache::Retention,
}

impl Context {
    pub fn new(device: &wgpu::Device, policy: CachePolicy) -> Self {
        Self {
            device: device.clone(),
            programs: Arc::default(),
            retention: cache::Retention::new(policy.generated_bytes),
        }
    }

    /// A bounded caller-owned lifetime for optional completed intermediates. Its payloads
    /// share this context's retention allowance with generated values.
    pub fn image_cache(&self, capacity: usize) -> ImageCache {
        ImageCache::new(self.programs.clone(), self.retention.clone(), capacity)
    }

    pub fn cache_policy(&self) -> CachePolicy {
        CachePolicy {
            generated_bytes: self.retention.limit(),
        }
    }

    /// Payload bytes owned by admitted reusable values, not total GPU allocation.
    pub fn retained_bytes(&self) -> u64 {
        self.retention.bytes()
    }

    pub fn request(
        &self,
        image: Arc<ImageNode>,
        region: Region,
        budget: Budget,
    ) -> Result<Evaluation, Error> {
        if image.domain.grid != region.grid {
            return Err(Error::InvalidParameter(
                "requested grid differs from composed image",
            ));
        }
        Ok(Evaluation {
            job: Job::new(image, region.bounds, budget)?.with_policy(Policy::Composed),
        })
    }

    pub(crate) fn require_device(&self, device: &wgpu::Device) -> Result<(), Error> {
        if device != &self.device {
            return Err(Error::InvalidParameter(
                "evaluation belongs to a different GPU device",
            ));
        }
        Ok(())
    }

    /// Compile declared shader work without acquiring or allocating its image resources.
    pub(crate) fn prewarm(&self, work: &[Work]) -> Result<(), Error> {
        self.executor().compile_work(&self.device, work)
    }

    /// Prepare component-generated code without acquiring pixels or allocating working images.
    #[cfg(feature = "ui")]
    pub(crate) fn prewarm_image(&self, image: Arc<ImageNode>) -> Result<(), Error> {
        let domain = image.domain;
        let evaluation = self.request(
            image,
            Region {
                bounds: domain.bounds,
                grid: domain.grid,
            },
            Budget {
                tile: [domain.bounds.width, domain.bounds.height],
                scratch_bytes: self.device.limits().max_buffer_size,
            },
        )?;
        for plan in evaluation.plans() {
            for tile in plan?.tiles {
                self.prewarm(&tile.fragment.work)?;
            }
        }
        Ok(())
    }

    pub(crate) fn executor(&self) -> Executor {
        Executor::with_retention(self.programs.clone(), self.retention.clone())
    }

    pub(crate) fn record(
        &self,
        evaluation: Evaluation,
        previous: Vec<Arc<RetainedBuffer>>,
    ) -> Result<Recorded, Error> {
        let mut plans = evaluation.plans();
        let plan = plans
            .next()
            .ok_or(Error::InvalidParameter("empty recorded evaluation"))??;
        if plans.next().is_some() {
            return Err(Error::InvalidParameter(
                "record-only execution requires one bounded tile",
            ));
        }
        self.executor().prepare(&self.device, plan, previous)
    }

    pub(crate) fn record_work(
        &self,
        fragment: Fragment,
        result: PlannedImage,
    ) -> Result<Recorded, Error> {
        self.executor().prepare(
            &self.device,
            Plan::from_work(fragment, result, self.device.limits().max_buffer_size)?,
            vec![],
        )
    }

    pub(crate) fn encode_work(
        &self,
        fragment: Fragment,
        result: PlannedImage,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &[&wgpu::Buffer],
    ) -> Result<(), Error> {
        self.record_work(fragment, result)?
            .encode_work(&self.device, encoder, inputs)
    }

    pub(crate) fn submit(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        encoder: wgpu::CommandEncoder,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<()>, Error> {
        self.submit_with(queue, gate, encoder, cancelled, || {})
    }

    pub(crate) fn submit_with(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        encoder: wgpu::CommandEncoder,
        cancelled: impl Fn() -> bool,
        submitted: impl FnOnce(),
    ) -> Result<Option<()>, Error> {
        if cancelled() {
            return Ok(None);
        }
        execute::submit(&self.device, queue, gate, encoder, submitted)?;
        Ok((!cancelled()).then_some(()))
    }

    /// Evaluate captured source/composition pixels on their declared grid into bounded GPU sinks.
    /// Sinks own publication; callers keep their writes private until successful completion.
    pub fn evaluate(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        evaluation: &Evaluation,
        cancelled: impl Fn() -> bool,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<Stats>, Error> {
        self.evaluate_with(
            queue,
            gate,
            evaluation,
            cancelled,
            Sink {
                encode: sink,
                submitted: || {},
            },
        )
    }

    /// Arm sink readback after each submission, before waiting for that bounded unit.
    pub fn evaluate_with<E, S>(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        evaluation: &Evaluation,
        cancelled: impl Fn() -> bool,
        mut sink: Sink<E, S>,
    ) -> Result<Option<Stats>, Error>
    where
        E: FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
        S: FnMut(),
    {
        let mut executor = self.executor();
        let mut total = Stats::default();
        let mut plans = evaluation.plans();
        loop {
            if cancelled() {
                return Ok(None);
            }
            let Some(plan) = plans.next() else {
                break;
            };
            let Some(stats) = executor.run_sink_with(
                &self.device,
                queue,
                gate,
                &plan?,
                &cancelled,
                Sink {
                    encode: &mut sink.encode,
                    submitted: &mut sink.submitted,
                },
            )?
            else {
                return Ok(None);
            };
            total.submissions += stats.submissions;
            total.source_cells += stats.source_cells;
            total.buffer_allocations += stats.buffer_allocations;
            total.texture_allocations += stats.texture_allocations;
            total.initializations += stats.initializations;
            total.encoded_device_peak = total.encoded_device_peak.max(stats.encoded_device_peak);
            total.scratch_bytes = total.scratch_bytes.max(stats.scratch_bytes);
        }
        Ok(Some(total))
    }
}

/// An immutable requested region of a source-bearing captured composition, lowered lazily.
pub struct Evaluation {
    job: Job,
}

impl Evaluation {
    pub(crate) fn plans(&self) -> impl Iterator<Item = Result<Plan, Error>> + '_ {
        self.job.plans()
    }
}
