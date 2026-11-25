//! Drawable image requests resolve together or retain their plans for worker execution.
use super::*;
use crate::gpu::allocation::DeviceAllocationExt;

#[cfg(test)]
mod tests;

pub(in crate::evaluation) struct Preparation {
    evaluation: Evaluation,
    work: PreparedWork,
}

enum PreparedWork {
    Cached(Image<ImageLayout>),
    Gpu(Plan),
    Produce(Plan),
    Units,
}

pub(crate) struct RecordedImage {
    pub pixels: BufferImage,
    pub commands: Option<Recorded>,
}

impl TryFrom<Image<ImageLayout>> for RecordedImage {
    type Error = Error;

    fn try_from(image: Image<ImageLayout>) -> Result<Self, Error> {
        Ok(Self {
            pixels: image.try_into()?,
            commands: None,
        })
    }
}

pub(crate) enum ImageDelivery {
    Drawable(Vec<RecordedImage>),
    Pending(PendingImages),
}

/// Owns the planned batch. Dropping it without polling `finish` starts no work.
pub(crate) struct PendingImages {
    context: Context,
    plans: Vec<Preparation>,
}

impl PendingImages {
    pub(crate) async fn finish(
        self,
        queue: &crate::gpu::submission::Queue,
        cancelled: impl Fn() -> bool,
    ) -> Result<Vec<RecordedImage>, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let images = self.context.produce_images(queue, self.plans)?;
        let mut images = Box::pin(futures_util::future::try_join_all(images));
        let completed = futures_util::future::poll_fn(|cx| {
            if cancelled() {
                return std::task::Poll::Ready(Err(Error::Cancelled));
            }
            images
                .as_mut()
                .poll(cx)
                .map(|result| result.map_err(|error| (*error).clone()))
        })
        .await?;
        completed.into_iter().map(RecordedImage::try_from).collect()
    }
}

impl Preparation {
    fn new(evaluation: Evaluation, work: PreparedWork) -> Self {
        Self { evaluation, work }
    }

    fn recording_passes(&self) -> Option<usize> {
        let passes = match &self.work {
            PreparedWork::Cached(_) => 0,
            PreparedWork::Gpu(plan) => plan.recording_passes(),
            PreparedWork::Produce(_) | PreparedWork::Units => return None,
        };
        (passes <= Plan::MAX_RECORDING_PASSES).then_some(passes)
    }

    pub(in crate::evaluation) fn can_record(&self) -> bool {
        self.recording_passes().is_some()
    }

    pub(in crate::evaluation) fn record(self, context: &Context) -> Result<RecordedImage, Error> {
        if !self.can_record() {
            return Err(Error::InvalidParameter(
                "image preparation requires production before recording",
            ));
        }

        match self.work {
            PreparedWork::Cached(image) => image.try_into(),
            PreparedWork::Gpu(plan) => {
                let root = self.evaluation.job.image();
                let region = self.evaluation.job.region();
                let commands = context
                    .executor()
                    .prepare(context.device(), plan)?
                    .with_request(root, region);
                let pixels = BufferImage::recorded(root, region, output_buffer(context, region)?)?;
                #[cfg(test)]
                if let BufferImage::Produced(image) = &pixels {
                    image.image.observe_inputs(commands.image_inputs());
                }
                Ok(RecordedImage {
                    pixels,
                    commands: Some(commands),
                })
            }
            PreparedWork::Produce(_) | PreparedWork::Units => Err(Error::InvalidParameter(
                "image preparation requires production before recording",
            )),
        }
    }

    fn produce(self, worker: &Worker<'_>, layout: ImageLayout) -> Result<Arc<ImageValue>, Error> {
        let plan = match self.work {
            PreparedWork::Cached(image) => return Ok(image.image),
            PreparedWork::Gpu(plan) | PreparedWork::Produce(plan) => plan,
            PreparedWork::Units => {
                return worker.materialize_image(
                    &self.evaluation,
                    layout,
                    &mut WorkCounts::default(),
                );
            }
        };
        worker.working.admit_gpu(worker.context.device())?;
        let pixels = output_buffer(worker.context, layout.region)?;
        Production {
            worker: *worker,
            layout,
            pixels: &pixels,
        }
        .deliver(&mut worker.executor(), plan)?;
        Ok(ImageValue::new(pixels))
    }
}

fn output_buffer(context: &Context, region: CellRegion) -> Result<wgpu::Buffer, Error> {
    context.device.try_create_buffer(&wgpu::BufferDescriptor {
        label: Some("prepared image result"),
        size: u64::from(region.width) * u64::from(region.height) * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Context {
    /// Plan a batch for the next draw. If any image needs a worker, defer the
    /// whole batch without starting work or recording the resident subset.
    pub(crate) fn request_images(
        &self,
        evaluations: Vec<Evaluation>,
    ) -> Result<ImageDelivery, Error> {
        let plans = self.prepare_images(evaluations)?;
        if plans.iter().all(Preparation::can_record) {
            let images = plans
                .into_iter()
                .map(|plan| plan.record(self))
                .collect::<Result<_, _>>()?;
            Ok(ImageDelivery::Drawable(images))
        } else {
            Ok(ImageDelivery::Pending(PendingImages {
                context: self.clone(),
                plans,
            }))
        }
    }

    /// Resolve an image inline on an existing blocking worker; never queue child work.
    pub(crate) fn request_image_on_worker(
        &self,
        evaluation: Evaluation,
        queue: &crate::gpu::submission::Queue,
        cancelled: impl Fn() -> bool,
    ) -> Result<RecordedImage, Error> {
        let plan = self.prepare_image(evaluation)?;
        if plan.can_record() {
            plan.record(self)
        } else {
            self.produce_prepared(queue, plan, cancelled)?.try_into()
        }
    }

    #[cfg(test)]
    pub(crate) fn record_supplied(
        &self,
        mut evaluation: Evaluation,
        supplied: Vec<SuppliedImage>,
    ) -> Result<RecordedImage, Error> {
        evaluation.job = evaluation.job.with_supplied(supplied.into());
        self.prepare_image(evaluation)?.record(self)
    }

    #[cfg(test)]
    pub(crate) fn produce_image_on_worker(
        &self,
        evaluation: Evaluation,
        queue: &crate::gpu::submission::Queue,
    ) -> Result<RecordedImage, Error> {
        self.produce_prepared(queue, self.prepare_image(evaluation)?, || false)?
            .try_into()
    }

    fn prepared_layout(&self, evaluation: &Evaluation) -> Result<ImageLayout, Error> {
        let layout = ImageLayout {
            domain: evaluation.job.image().domain(),
            region: evaluation.job.region(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let limits = self.device.limits();
        if layout.bytes() > limits.max_buffer_size
            || layout.bytes() > limits.max_storage_buffer_binding_size
        {
            return Err(Error::EvaluationLimit("prepared image exceeds GPU storage"));
        }
        Ok(layout)
    }

    pub(in crate::evaluation) fn prepare_image(
        &self,
        evaluation: Evaluation,
    ) -> Result<Preparation, Error> {
        let evaluation = evaluation.with_materializations(self.materializations());
        let root = evaluation.job.image();
        let region = evaluation.job.region();
        let layout = self.prepared_layout(&evaluation)?;
        let limits = self.device.limits();
        self.products.requested(root, region);
        let cached = root.output(region)?.identity().and_then(|key| {
            self.products
                .ready::<ImageLayout>(key)
                .into_iter()
                .find(|image| *image.descriptor == layout)
        });
        let work = if let Some(image) = cached {
            self.products.used(&image.image);
            self.products.publish(&image, evaluation.retain);
            PreparedWork::Cached(image)
        } else {
            match evaluation.job.probe(region) {
                Ok(plan) if !plan.requires_production(root)? => {
                    match Executor::admit(&limits, &plan) {
                        Ok(()) => {
                            let missing =
                                plan.tiles.iter().flat_map(|tile| &tile.fragment.work).any(
                                    |work| {
                                        matches!(work, Work::Produce { .. } | Work::Upload { .. })
                                    },
                                );
                            if missing {
                                PreparedWork::Produce(plan)
                            } else {
                                PreparedWork::Gpu(plan)
                            }
                        }
                        Err(Error::EvaluationLimit(_)) => PreparedWork::Units,
                        Err(error) => return Err(error),
                    }
                }
                Ok(_) | Err(Error::EvaluationLimit(_)) => PreparedWork::Units,
                Err(error) => return Err(error),
            }
        };
        Ok(Preparation::new(evaluation, work))
    }

    pub(super) fn prepare_images(
        &self,
        evaluations: Vec<Evaluation>,
    ) -> Result<Vec<Preparation>, Error> {
        evaluations
            .into_iter()
            .map(|evaluation| self.prepare_image(evaluation))
            .collect()
    }

    /// Fulfil a captured preparation on an existing blocking worker, without queueing child work.
    /// This submits and waits; presentation callbacks must use recording or async production.
    pub(in crate::evaluation) fn produce_prepared(
        &self,
        queue: &crate::gpu::submission::Queue,
        preparation: Preparation,
        cancelled: impl Fn() -> bool,
    ) -> Result<Image<ImageLayout>, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let evaluation = &preparation.evaluation;
        let definition = evaluation.job.image();
        let region = evaluation.job.region();
        let identity =
            definition
                .output(region)?
                .identity()
                .cloned()
                .ok_or(Error::InvalidParameter(
                    "prepared image requires captured content",
                ))?;
        let layout = ImageLayout {
            domain: definition.domain(),
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let retain = evaluation.retain;
        let working = RunWorkingSet::default();
        let scope = self.materializations();
        let worker = Worker {
            context: self,
            queue,
            scope: &scope,
            working: &working,
            cancelled: &cancelled,
            progress: &|| {},
        };
        let value = preparation.produce(&worker, layout)?;
        working.finish_gpu()?;
        let image = Image {
            identity,
            descriptor: Arc::new(layout),
            image: value,
        };
        scope.publish(&image, retain);
        Ok(image)
    }

    pub(in crate::evaluation) fn produce_images(
        &self,
        queue: &crate::gpu::submission::Queue,
        prepared: Vec<Preparation>,
    ) -> Result<Vec<ImageFuture<ImageLayout>>, Error> {
        let scope = self.materializations();
        let mut requests = Vec::new();
        for preparation in &prepared {
            let evaluation = &preparation.evaluation;
            let value = evaluation.job.image();
            let region = evaluation.job.region();
            let identity =
                value
                    .output(region)?
                    .identity()
                    .cloned()
                    .ok_or(Error::InvalidParameter(
                        "prepared image requires captured content",
                    ))?;
            let layout = ImageLayout {
                domain: value.domain(),
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            requests.push((
                identity,
                Arc::new(layout),
                layout.bytes(),
                evaluation.request_policy(),
            ));
        }
        let (context, queue) = (self.clone(), queue.clone());
        let owned_scope = scope.clone();
        Ok(scope.obtain_many_for(requests, move |missing| async move {
            let progress = Arc::new(futures_util::task::AtomicWaker::new());
            let wake = progress.clone();
            let mut job = Box::pin(crate::background::run(move |stop| {
                let working = RunWorkingSet::default();
                let cancelled = || stop.is_cancelled();
                let progress = || wake.wake();
                if missing.len() > 1 {
                    let jobs = missing
                        .iter()
                        .map(|index| {
                            let evaluation = prepared[*index].evaluation.clone();
                            let layout = ImageLayout {
                                domain: evaluation.job.image().domain(),
                                region: evaluation.job.region(),
                                format: ImageFormat::Rgba32Float,
                                backing: ImageBacking::Buffer,
                            };
                            (evaluation, layout)
                        })
                        .collect::<Vec<_>>();
                    let selected = jobs.iter().collect::<Vec<_>>();
                    if !jobs.is_empty() {
                        let worker = Worker {
                            context: &context,
                            queue: &queue,
                            scope: &owned_scope,
                            working: &working,
                            cancelled: &cancelled,
                            progress: &progress,
                        };
                        if let Some(images) = worker.materialize_units_many(&selected)? {
                            working.finish_gpu()?;
                            return Ok(images);
                        }
                    }
                }
                let mut images = Vec::new();
                for (index, preparation) in prepared.into_iter().enumerate() {
                    if !missing.contains(&index) {
                        continue;
                    }
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let evaluation = &preparation.evaluation;
                    let layout = ImageLayout {
                        domain: evaluation.job.image().domain(),
                        region: evaluation.job.region(),
                        format: ImageFormat::Rgba32Float,
                        backing: ImageBacking::Buffer,
                    };
                    let worker = Worker {
                        context: &context,
                        queue: &queue,
                        scope: &owned_scope,
                        working: &working,
                        cancelled: &cancelled,
                        progress: &progress,
                    };
                    let identity = evaluation
                        .job
                        .image()
                        .output(layout.region)?
                        .identity()
                        .cloned()
                        .unwrap();
                    let retain = evaluation.retain;
                    let image = preparation.produce(&worker, layout)?;
                    owned_scope.publish(
                        &Image {
                            identity,
                            descriptor: Arc::new(layout),
                            image: image.clone(),
                        },
                        retain,
                    );
                    images.push(image);
                }
                working.finish_gpu()?;
                Ok(images)
            }));
            futures_util::future::poll_fn(|cx| {
                progress.register(cx.waker());
                std::future::Future::poll(job.as_mut(), cx)
            })
            .await
            .map_err(|_| Error::Gpu("image preparation worker panicked".into()))?
        }))
    }
}
