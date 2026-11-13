//! Device-scoped evaluation policy; optional retention is separate from required work budgets.

use super::CellRegion;
use super::*;

mod gpu;
mod production;
mod readback;
mod unit;
pub use production::{Production, Worker};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    /// Optional image and generated-resource payload bytes across evaluation scopes.
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
    pub bounds: CellRegion,
    pub grid: Grid,
}

#[cfg(feature = "editor")]
mod batch;
#[cfg(feature = "editor")]
mod batch_units;
mod materialize;

#[derive(Clone)]
pub struct Context {
    device: wgpu::Device,
    pub(crate) programs: Arc<Programs>,
    retention: cache::Allowance,
    #[cfg(feature = "editor")]
    pub(super) active_inputs: cache::Allowance,
    products: Materializations,
}

impl Context {
    pub fn new(device: &wgpu::Device, policy: CachePolicy) -> Self {
        let programs = Arc::new(Programs::default());
        let retention = cache::Allowance::new(policy.generated_bytes);
        Self {
            device: device.clone(),
            products: Materializations::new(programs.clone(), retention.clone()),
            programs,
            retention,
            #[cfg(feature = "editor")]
            active_inputs: cache::Allowance::new(super::inputs::capacity(device)),
        }
    }

    /// Isolated stores for reservation, eviction and supplied-value fixtures.
    #[cfg(test)]
    pub fn test_materializations(&self, retain: bool) -> Materializations {
        Materializations::new(
            self.programs.clone(),
            if retain {
                self.retention.clone()
            } else {
                cache::Allowance::new(0)
            },
        )
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn cache_policy(&self) -> CachePolicy {
        CachePolicy {
            generated_bytes: self.retention.limit(),
        }
    }

    #[cfg(feature = "editor")]
    pub(crate) fn set_retention_limit(&self, bytes: u64) {
        if self.retention.limit() != bytes {
            self.products.set_retention_limit(bytes);
        }
    }

    /// Shared in-flight production, live-value discovery and byte-bounded optional retention.
    pub fn materializations(&self) -> Materializations {
        self.products.clone()
    }

    /// Payload bytes owned by admitted reusable values, not total GPU allocation.
    pub fn retained_bytes(&self) -> u64 {
        self.retention.bytes()
    }

    pub fn request(
        &self,
        image: impl Into<value::Value>,
        region: Region,
        budget: Budget,
    ) -> Result<Evaluation, Error> {
        let image = image.into();
        if image.shape() != value::Shape::Pixel {
            return Err(Error::InvalidParameter(
                "record values cannot be requested as photographic images",
            ));
        }
        if image.domain.grid != region.grid {
            return Err(Error::InvalidParameter(
                "requested grid differs from composed image",
            ));
        }
        Ok(Evaluation {
            retain: true,
            job: Job::new(image, region.bounds, budget)?
                .with_limits(self.device.limits())
                .with_policy(Policy::Composed)
                .with_materializations(self.materializations()),
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
    pub(crate) fn prewarm_image(&self, image: Value) -> Result<(), Error> {
        let domain = image.domain;
        let bounds = domain.finite()?;
        let evaluation = self.request(
            image,
            Region {
                bounds,
                grid: domain.grid,
            },
            Budget {
                tile: [bounds.width, bounds.height],
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
        Executor::with_scope(self.programs.clone(), self.materializations())
    }

    pub(crate) fn record(
        &self,
        mut evaluation: Evaluation,
        scope: Materializations,
        supplied: Vec<SuppliedImage>,
    ) -> Result<Recorded, Error> {
        evaluation.job = evaluation.job.with_supplied(supplied.into());
        if !scope.belongs_to(self) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        let evaluation = evaluation.with_materializations(scope.clone());
        let mut plans = evaluation.plans();
        let plan = plans
            .next()
            .ok_or(Error::InvalidParameter("empty recorded evaluation"))??;
        if plans.next().is_some() {
            return Err(Error::InvalidParameter(
                "record-only execution requires one bounded tile",
            ));
        }
        Ok(Executor::with_scope(self.programs.clone(), scope)
            .prepare(&self.device, plan)?
            .with_request(evaluation.job.image(), evaluation.job.region()))
    }

    #[cfg(feature = "render")]
    pub(crate) fn prewarm_texture(&self, image: Value) -> Result<(), Error> {
        let region = image.domain().finite()?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("attachment program preparation"),
            size: wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let job = Job::new(
            image,
            region,
            Budget {
                tile: [region.width, region.height],
                scratch_bytes: self.device.limits().max_buffer_size,
            },
        )?
        .with_policy(Policy::Composed)
        .with_limits(self.device.limits());
        for tile in job.texture_plan(&texture)?.tiles {
            self.prewarm(&tile.fragment.work)?;
        }
        Ok(())
    }

    #[cfg(feature = "render")]
    pub(crate) fn record_to_texture(
        &self,
        mut evaluation: Evaluation,
        scope: Materializations,
        supplied: Vec<SuppliedImage>,
        texture: &wgpu::Texture,
    ) -> Result<Recorded, Error> {
        if !scope.belongs_to(self) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        evaluation.job = evaluation
            .job
            .with_supplied(supplied.into())
            .with_materializations(scope.clone());
        let plan = evaluation.job.texture_plan(texture)?;
        Ok(Executor::with_scope(self.programs.clone(), scope)
            .prepare(&self.device, plan)?
            .with_request(evaluation.job.image(), evaluation.job.region()))
    }

    pub(crate) fn record_work(
        &self,
        fragment: Fragment,
        result: PlannedImage,
    ) -> Result<Recorded, Error> {
        self.executor().prepare(
            &self.device,
            Plan::from_work(fragment, result, self.device.limits().max_buffer_size)?,
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
            .map(|_| ())
    }

    pub(crate) fn submit(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
        encoder: wgpu::CommandEncoder,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<()>, Error> {
        self.submit_with(queue, gate, encoder, cancelled, || {})
    }

    pub(crate) fn submit_with(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
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
        gate: &crate::gpu::submission::SubmissionGate,
        evaluation: &Evaluation,
        cancelled: impl Fn() -> bool,
        sink: impl FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
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
        gate: &crate::gpu::submission::SubmissionGate,
        evaluation: &Evaluation,
        cancelled: impl Fn() -> bool,
        mut sink: Sink<E, S>,
    ) -> Result<Option<WorkCounts>, Error>
    where
        E: FnMut(&mut wgpu::CommandEncoder, PlannedImage, &wgpu::Buffer) -> Result<(), Error>,
        S: FnMut(),
    {
        if cancelled() {
            return Ok(None);
        }
        let scope = evaluation
            .job
            .materializations()
            .cloned()
            .unwrap_or_else(|| self.materializations());
        scope.requested(evaluation.job.image(), evaluation.job.region());
        let mut executor = Executor::with_scope(self.programs.clone(), scope);
        let mut total = WorkCounts::default();
        let mut plans = evaluation.plans();
        loop {
            if cancelled() {
                return Ok(None);
            }
            let Some(plan) = plans.next() else {
                break;
            };
            let Some(stats) = self.run_sink(
                &mut executor,
                (queue, gate),
                &plan?,
                (&cancelled, &|| {}),
                Sink {
                    encode: &mut sink.encode,
                    submitted: &mut sink.submitted,
                },
            )?
            else {
                return Ok(None);
            };
            total.include(stats);
        }
        Ok(Some(total))
    }
}

/// An immutable requested region of a source-bearing captured composition, lowered lazily.
#[derive(Clone)]
pub struct Evaluation {
    job: Job,
    retain: bool,
}

impl Evaluation {
    /// Keep this result through its consumers, without requesting optional cache ownership.
    /// Independent requests and declared dependency units can still retain their values.
    pub fn without_retention(mut self) -> Self {
        self.retain = false;
        self
    }

    pub(super) fn request_policy(&self) -> super::materializations::RequestPolicy {
        super::materializations::RequestPolicy {
            budget: Some(self.job.budget()),
            retain: self.retain,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_policy(mut self, policy: Policy) -> Self {
        self.job = self.job.with_policy(policy);
        self
    }

    pub fn image(&self) -> &value::Value {
        self.job.image()
    }

    pub fn region(&self) -> Region {
        Region {
            bounds: self.job.region(),
            grid: self.job.image().domain().grid,
        }
    }

    pub(in crate::gpu::evaluation) fn with_materializations(
        mut self,
        scope: Materializations,
    ) -> Self {
        self.job = self.job.with_materializations(scope);
        self
    }

    pub(crate) fn plans(&self) -> impl Iterator<Item = Result<Plan, Error>> + '_ {
        self.job.plans()
    }
}
