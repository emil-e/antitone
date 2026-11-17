//! Actual image demands retain their preparation through recording or missing production.
use super::*;
use crate::gpu::evaluation::materializations::ProducedImage;

mod sharing;
#[cfg(test)]
mod tests;

pub(crate) struct Preparation {
    evaluation: Evaluation,
    work: PreparedWork,
    destination: Option<BufferImage>,
    dependencies: Vec<Preparation>,
    // Keep discovered physical versions available until execution takes over their leases.
    captured_inputs: Vec<Arc<ImageValue>>,
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
    pub(crate) fn can_record(&self) -> bool {
        matches!(self.work, PreparedWork::Cached(_) | PreparedWork::Gpu(_))
            && self.dependencies.iter().all(Self::can_record)
    }

    fn scratch_bytes(&self) -> u64 {
        match &self.work {
            PreparedWork::Gpu(plan) | PreparedWork::Produce(plan) => plan.peak_scratch_bytes,
            _ => 0,
        }
    }

    pub(crate) fn record(self, context: &Context) -> Result<RecordedImage, Error> {
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
                    None => BufferImage::recorded(root, region, output_buffer(context, region))?,
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
        let plan = match self.work {
            PreparedWork::Cached(image) => {
                return Ok(ProducedImage {
                    image: image.image,
                    inputs: image.inputs,
                });
            }
            PreparedWork::Gpu(plan) | PreparedWork::Produce(plan) => plan,
            PreparedWork::Units => {
                let worker = Worker {
                    budget: self.evaluation.job.budget(),
                    ..*worker
                };
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
        let mut owners = Vec::new();
        for dependency in self.dependencies {
            let definition = dependency.evaluation.job.image().clone();
            let region = dependency.evaluation.job.region();
            let layout = ImageLayout {
                domain: definition.domain(),
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            let own = Worker {
                budget: dependency.evaluation.job.budget(),
                ..*worker
            };
            let image = dependency.produce(&own, layout)?;
            owners.extend(image.inputs.iter().cloned());
            owners.push(image.image.clone());
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
        worker
            .working
            .admit_gpu(worker.context.device(), 0, worker.budget.scratch_bytes)?;
        let image = match self.destination {
            Some(BufferImage::Produced(image)) => image.image,
            Some(BufferImage::Submitted(pixels)) => ImageValue::new(pixels),
            None => ImageValue::new(output_buffer(worker.context, layout.region)),
        };
        let pixels = image.as_buffer().expect("prepared buffer destination");
        let _target = worker.working.activate_buffer(pixels);
        // New operand outputs remain live together, unlike ordinary scratch-slot reuse.
        // Reserve them before giving any nested producer or the final plan its allowance.
        let produced_bytes = plan
            .tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .filter_map(|step| match step {
                Work::Produce { output, .. } | Work::Upload { output, .. } => {
                    Some(u64::from(output.region.width) * u64::from(output.region.height) * 16)
                }
                _ => None,
            })
            .try_fold(0u64, u64::checked_add);
        let available = produced_bytes
            .and_then(|bytes| worker.budget.scratch_bytes.checked_sub(bytes))
            .filter(|available| *available >= plan.peak_scratch_bytes && *available > 0);
        let Some(available) = available else {
            Production {
                worker: *worker,
                layout,
                pixels,
            }
            .deliver(&mut worker.executor(), plan)?;
            return Ok(ProducedImage::new(image, owners));
        };
        let producer = Worker {
            budget: Budget {
                scratch_bytes: available,
                ..worker.budget
            },
            ..*worker
        };
        let tile = plan
            .tiles
            .into_iter()
            .next()
            .ok_or(Error::InvalidParameter(
                "prepared image has no execution tile",
            ))?;
        let mut fragment = tile.fragment;
        let mut work = Vec::new();
        for step in std::mem::take(&mut fragment.work) {
            (worker.progress)();
            if (worker.cancelled)() {
                return Err(Error::Cancelled);
            }
            let (image, output) = match step {
                Work::Produce { image, output } => (image, output),
                Work::Upload { source, output } => (super::super::source(source), output),
                other => {
                    work.push(other);
                    continue;
                }
            };
            let input_layout = ImageLayout {
                domain: output.domain,
                region: output.region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            let query = producer.query(image.clone(), output.region, available)?;
            let pixels =
                producer.materialize_image(&query, input_layout, &mut WorkCounts::default())?;
            if let Some(key) = image.output(output.region)?.identity() {
                let root = self.evaluation.job.image().output(layout.region)?;
                let retain = self.evaluation.retain
                    || output.region != layout.region
                    || output.domain != layout.domain
                    || root.identity() != Some(key);
                worker.scope.publish(
                    &Image {
                        identity: key.clone(),
                        descriptor: Arc::new(input_layout),
                        image: pixels.clone(),
                        inputs: pixels.reusable_inputs(),
                    },
                    retain,
                );
            }
            owners.extend(pixels.reusable_inputs().iter().cloned());
            owners.push(pixels.clone());
            fragment.resources[output.resource.0] = Storage::ResidentBuffer(pixels);
        }
        fragment.work = work;
        // Production becoming an immutable input changes slot liveness, not mathematics.
        let plan = Plan::from_work(fragment, tile.result, available)?;
        let recorded = worker
            .executor()
            .prepare(worker.context.device(), plan)?
            .with_request(self.evaluation.job.image(), layout.region);
        owners.extend(recorded.reusable_inputs());
        let mut encoder = worker
            .context
            .device
            .create_command_encoder(&Default::default());
        recorded.encode_result(worker.context.device(), &mut encoder, pixels)?;
        worker
            .context
            .submit_with(worker.queue, worker.gate, encoder, worker.cancelled, || {
                recorded.submitted()
            })?
            .ok_or(Error::Cancelled)?;
        Ok(ProducedImage::new(image, owners))
    }
}

fn output_buffer(context: &Context, region: CellRegion) -> wgpu::Buffer {
    context.device.create_buffer(&wgpu::BufferDescriptor {
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

    fn prepare_base(&self, evaluation: Evaluation) -> Result<Preparation, Error> {
        let evaluation = evaluation.with_materializations(self.materializations());
        let root = evaluation.job.image();
        let region = evaluation.job.region();
        let layout = ImageLayout {
            domain: root.domain(),
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let limits = self.device.limits();
        if layout.bytes() > limits.max_buffer_size
            || layout.bytes() > limits.max_storage_buffer_binding_size
        {
            return Err(Error::WorkingAllowance(
                "prepared image exceeds GPU storage",
            ));
        }
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
                Ok(plan)
                    if !plan.requires_production(root)?
                        && plan.peak_scratch_bytes <= evaluation.job.budget().scratch_bytes =>
                {
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
                        Err(Error::WorkingAllowance(_)) => PreparedWork::Units,
                        Err(error) => return Err(error),
                    }
                }
                Ok(_) | Err(Error::WorkingAllowance(_)) => PreparedWork::Units,
                Err(error) => return Err(error),
            }
        };
        Ok(Preparation {
            evaluation,
            work,
            destination: None,
            dependencies: Vec::new(),
            captured_inputs: Vec::new(),
        })
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
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
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
            gate,
            scope: &scope,
            budget: evaluation.job.budget(),
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
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
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
        let (context, queue, gate) = (self.clone(), queue.clone(), gate.clone());
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
                        if let Some((evaluation, _)) = jobs.first() {
                            let worker = Worker {
                                context: &context,
                                queue: &queue,
                                gate: &gate,
                                scope: &owned_scope,
                                working: &working,
                                budget: evaluation.job.budget(),
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
                            gate: &gate,
                            scope: &owned_scope,
                            working: &working,
                            budget: evaluation.job.budget(),
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
