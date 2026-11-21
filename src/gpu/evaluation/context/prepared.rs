//! Actual image demands retain their preparation through recording or missing production.
use super::*;
use crate::gpu::allocation::DeviceAllocationExt;
use crate::gpu::evaluation::materializations::ProducedImage;

mod sharing;
#[cfg(test)]
mod tests;

pub(crate) struct Preparation {
    evaluation: Evaluation,
    work: PreparedWork,
    destination: Option<BufferImage>,
    dependencies: Vec<Preparation>,
    kind: DependencyKind,
    // Keep discovered physical versions available until execution takes over their leases.
    captured_inputs: Vec<Arc<ImageValue>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DependencyKind {
    Shared,
    SourceProjection,
    // Acquisition storage belongs to execution, not to the independently completed output.
    SourceSlice,
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

impl Preparation {
    fn new(evaluation: Evaluation, work: PreparedWork) -> Self {
        Self {
            evaluation,
            work,
            destination: None,
            dependencies: Vec::new(),
            kind: DependencyKind::Shared,
            captured_inputs: Vec::new(),
        }
    }

    fn recording_passes(&self) -> Option<usize> {
        let own = match &self.work {
            PreparedWork::Cached(_) => 0,
            PreparedWork::Gpu(plan) => plan.recording_passes(),
            PreparedWork::Produce(_) | PreparedWork::Units => return None,
        };
        self.dependencies
            .iter()
            .try_fold(own, |passes, dependency| {
                dependency
                    .recording_passes()
                    .map(|additional| passes + additional)
            })
            .filter(|passes| *passes <= Plan::MAX_RECORDING_PASSES)
    }

    pub(crate) fn can_record(&self) -> bool {
        self.recording_passes().is_some()
    }

    pub(crate) fn record(self, context: &Context) -> Result<RecordedImage, Error> {
        if !self.can_record() {
            return Err(Error::InvalidParameter(
                "image preparation requires production before recording",
            ));
        }

        let prerequisites = self
            .dependencies
            .into_iter()
            .map(|work| work.record(context))
            .collect::<Result<Vec<_>, _>>()?;
        match self.work {
            PreparedWork::Cached(image) => Ok(RecordedImage {
                pixels: image.try_into()?,
                commands: None,
            }),
            PreparedWork::Gpu(plan) => {
                let root = self.evaluation.job.image();
                let region = self.evaluation.job.region();
                let commands = context
                    .executor()
                    .prepare(context.device(), plan)?
                    .with_request(root, region)
                    .with_prerequisites(prerequisites);
                let pixels = match self.destination {
                    Some(pixels) => pixels,
                    None => BufferImage::recorded(root, region, output_buffer(context, region)?)?,
                };
                let pixels = match pixels {
                    BufferImage::Produced(mut image) => {
                        image.inputs =
                            ProducedImage::new(image.image.clone(), commands.reusable_inputs())
                                .inputs;
                        BufferImage::Produced(image)
                    }
                    pixels => pixels,
                };
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

    fn produce(self, worker: &Worker<'_>, layout: ImageLayout) -> Result<ProducedImage, Error> {
        // Recording/submission takes over physical ownership. Discovery is not a
        // reason to make every input a reusable dependency of the completed image.
        let _captured_inputs = self.captured_inputs;
        let mut execution_sources = Vec::new();
        let mut owners = Vec::new();
        for dependency in self.dependencies {
            let kind = dependency.kind;
            let definition = dependency.evaluation.job.image().clone();
            let region = dependency.evaluation.job.region();
            let layout = ImageLayout {
                domain: definition.domain(),
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            let own = Worker { ..*worker };
            let image = dependency.produce(&own, layout)?;
            if kind == DependencyKind::SourceSlice {
                // Keep acquired pixels through consumer execution, not the
                // completed producer's reusable upstream allocations.
                execution_sources.push(image.image.clone());
            } else {
                owners.extend(image.inputs.iter().cloned());
                owners.push(image.image.clone());
            }
            if let Some(identity) = definition.output(region)?.identity() {
                worker.scope.publish(
                    &Image {
                        identity: identity.clone(),
                        descriptor: Arc::new(layout),
                        image: image.image,
                        inputs: image.inputs,
                    },
                    true,
                );
            }
        }
        let plan = match self.work {
            PreparedWork::Cached(image) => {
                return Ok(ProducedImage {
                    image: image.image,
                    inputs: image.inputs,
                });
            }
            PreparedWork::Gpu(plan) | PreparedWork::Produce(plan) => plan,
            PreparedWork::Units => {
                let worker = Worker { ..*worker };
                if let Some(destination) = self.destination {
                    let image = match destination {
                        BufferImage::Produced(image) => image.image,
                        BufferImage::Submitted(pixels) => ImageValue::new(pixels),
                    };
                    Production {
                        worker,
                        layout,
                        pixels: image.as_buffer().expect("prepared buffer destination"),
                    }
                    .produce(&self.evaluation)?;
                    return Ok(ProducedImage::new(image, Vec::new()));
                }
                let image = worker.materialize_image(
                    &self.evaluation,
                    layout,
                    &mut WorkCounts::default(),
                )?;
                return Ok(image.into());
            }
        };
        worker.working.admit_gpu(worker.context.device())?;
        let image = match self.destination {
            Some(BufferImage::Produced(image)) => image.image,
            Some(BufferImage::Submitted(pixels)) => ImageValue::new(pixels),
            None => ImageValue::new(output_buffer(worker.context, layout.region)?),
        };
        Production {
            worker: *worker,
            layout,
            pixels: image.as_buffer().expect("prepared buffer destination"),
        }
        .deliver(&mut worker.executor(), plan)?;
        Ok(ProducedImage::new(image, owners))
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
    #[cfg(test)]
    pub(crate) fn prepare_supplied(
        &self,
        mut evaluation: Evaluation,
        supplied: Vec<SuppliedImage>,
    ) -> Result<Preparation, Error> {
        evaluation.job = evaluation.job.with_supplied(supplied.into());
        self.prepare_image(evaluation)
    }

    pub(crate) fn prepare_image(&self, evaluation: Evaluation) -> Result<Preparation, Error> {
        let root = evaluation.job.image();
        let region = evaluation.job.region();
        let cached = root.output(region)?.identity().is_some_and(|key| {
            self.products.ready::<ImageLayout>(key).iter().any(|image| {
                image.descriptor.domain == root.domain() && image.descriptor.region == region
            })
        });
        if !cached && let Some(prepared) = self.prepare_dependencies(&evaluation)? {
            return Ok(prepared);
        }
        self.prepare_base(evaluation)
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

    fn prepare_base(&self, evaluation: Evaluation) -> Result<Preparation, Error> {
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

    pub(crate) fn prepare_images(
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
    pub(crate) fn produce_prepared(
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
            image: value.image,
            inputs: value.inputs,
        };
        scope.publish(&image, retain);
        Ok(image)
    }

    pub(crate) fn produce_images(
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
        Ok(
            scope.obtain_many_owned(requests, move |missing| async move {
                let progress = Arc::new(futures_util::task::AtomicWaker::new());
                let wake = progress.clone();
                let mut job = Box::pin(crate::background::run(move |stop| {
                    let working = RunWorkingSet::default();
                    let cancelled = || stop.is_cancelled();
                    let progress = || wake.wake();
                    if missing.len() > 1
                        && missing
                            .iter()
                            .all(|index| prepared[*index].dependencies.is_empty())
                    {
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
                                return Ok(images.into_iter().map(ProducedImage::from).collect());
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
                                image: image.image.clone(),
                                inputs: image.inputs.clone(),
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
            }),
        )
    }
}
