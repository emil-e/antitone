use super::*;
use crate::scan::DevelopedSource;

enum Input {
    Proxy(Arc<Proxy>),
    Source(Arc<DevelopedSource>),
    Loading(
        std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Arc<DevelopedSource>, Error>> + Send>,
        >,
    ),
    Positive,
    Retired,
}

struct Work {
    job: Job,
    input: Result<Input, Error>,
    started: std::time::Instant,
    cache_hit: bool,
    metrics: ThumbnailMetrics,
}

struct Worker {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderers: Arc<crate::renderer::RendererCache>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    disk: Option<disk::DiskCache>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    developments: source_loading::SourceLoader,
    kernels: std::sync::OnceLock<Result<Arc<crate::gpu::view::ViewKernels>, Error>>,
}

impl Worker {
    fn active(&self, job: &Job) -> bool {
        self.generation.load(std::sync::atomic::Ordering::Relaxed) == job.ticket
            && !job.cancelled.is_cancelled()
    }

    fn begin(&self, job: Job) -> Work {
        let started = std::time::Instant::now();
        let native_key = job.key.source.store_key_at(&job.path);
        let cached = native_key.as_ref().and_then(|key| job.store.get_proxy(key));
        let cache_hit = cached.is_some();
        let mut metrics = ThumbnailMetrics::default();
        let input = (|| {
            if !self.active(&job) {
                return Ok(Input::Retired);
            }
            let positive = job.render && matches!(&job.key.recipe, Ok(Some(_)));
            if job.render {
                if let Err(reason) = &job.key.recipe {
                    return Err(Error::Gpu(reason.clone()));
                }
                if positive
                    && job
                        .previous
                        .as_ref()
                        .is_some_and(|product| product.positive.is_some())
                {
                    return Ok(Input::Positive);
                }
            }
            let cached = cached.or_else(|| {
                let disk = self.disk.as_ref()?;
                let tick = std::time::Instant::now();
                let result = disk.load(&job.key.source);
                metrics.disk_seconds = tick.elapsed().as_secs_f64();
                match result {
                    Ok(proxy) => {
                        metrics.disk_hit = proxy.is_some();
                        proxy
                    }
                    Err(error) => {
                        log::debug!(
                            source_id = job.source.id.0,
                            error = error.to_string().as_str();
                            "thumbnail_disk_cache_miss"
                        );
                        None
                    }
                }
            });
            if let Some(proxy) = cached {
                return Ok(Input::Proxy(proxy));
            }
            if !self.active(&job) {
                return Ok(Input::Retired);
            }
            let key = crate::source::SourceKey::for_load(
                &job.path,
                job.key.source.file.clone(),
                &job.source.development,
            );
            let loader = self.developments.clone();
            let cache = job.store.clone();
            let priority = job.priority;
            Ok(Input::Loading(Box::pin(async move {
                cache.load_source(&loader, key, priority).await
            })))
        })();
        Work {
            job,
            input,
            started,
            cache_hit,
            metrics,
        }
    }

    fn positive(
        &self,
        job: &Job,
        proxy: Option<&crate::scan::CameraProxy>,
        metrics: &mut ThumbnailMetrics,
    ) -> Result<Option<Arc<Product>>, Error> {
        use crate::gpu::view::{RenderedView, ViewKernels, ViewSource, ViewSourceSpec};
        let Some(recipe) = job
            .key
            .recipe
            .as_ref()
            .map_err(|error| Error::Gpu(error.clone()))?
            .as_ref()
        else {
            return Err(Error::Gpu("positive thumbnail requires a recipe".into()));
        };
        if !self.active(job) {
            return Ok(None);
        }
        let tick = std::time::Instant::now();
        let renderer = self.renderers.get(&self.device, &self.queue)?;
        metrics.renderer_seconds = tick.elapsed().as_secs_f64();
        let submissions = self.renderers.submissions();
        let previous = job
            .previous
            .as_ref()
            .and_then(|product| product.positive.as_ref());
        let source = if let Some((source, _)) = previous {
            metrics.neutral_hit = true;
            source.clone()
        } else {
            let proxy =
                proxy.ok_or_else(|| Error::Gpu("thumbnail requires a camera proxy".into()))?;
            let tick = std::time::Instant::now();
            let mut frame = crate::frame_raster::FrameRaster::full(Composition::new(
                job.source.development.active_area.into(),
                Default::default(),
                Default::default(),
            )?)?;
            frame.output = proxy.dimensions();
            let spec = ViewSourceSpec::new(
                job.source.id.0,
                frame,
                recipe.neutral.processing_input()?,
                proxy.positive_matrix(&recipe.neutral.processing_input()?)?,
                recipe.neutral.correction.clone(),
            )?;
            let kernels = self
                .kernels
                .get_or_init(|| pollster::block_on(ViewKernels::new(&self.device)))
                .clone()?;
            let source = ViewSource::from_camera_proxy(
                &self.device,
                &self.queue,
                submissions,
                renderer,
                kernels,
                proxy,
                job.correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
                spec,
            )?;
            metrics.neutral_seconds = tick.elapsed().as_secs_f64();
            source
        };
        let tick = std::time::Instant::now();
        let grade = Arc::new(crate::gpu::GpuGrade::new(
            &self.device,
            recipe.roll,
            recipe.grade.clone(),
            recipe.neutral.source_development.active_area.into(),
        )?);
        let grade = Arc::new(crate::gpu::GpuPipeline::compose(
            &self.device,
            source
                .input_program()
                .ok_or(Error::InvalidParameter(
                    "positive source program unavailable",
                ))?
                .clone(),
            grade,
            recipe.neutral.source_development.active_area.into(),
        )?);
        let input = crate::gpu::ProcessingInput::from(grade)
            .with_look(recipe.look.clone())
            .with_film_emulation(recipe.film_emulation)
            .with_frame_size(recipe.composition.crop().size)
            .with_film_seed(recipe.neutral.source.0);
        let view = RenderedView::new(
            &self.device,
            source.clone(),
            &input,
            job.key.target,
            previous.map(|(_, view)| view),
        )?;
        view.ensure_submitted(
            &self.device,
            &self.queue,
            submissions,
            self.repaint.get().unwrap_or(&egui::Context::default()),
        );
        if recipe.look.is_some() {
            // Cards retain a full-source raster; admit only their declared composed footprint.
            let composition = job.key.projection.unwrap_or(recipe.composition);
            let mapping = composition.texture_mapping(
                crate::geometry::FramePoint([0.; 2]),
                crate::geometry::FramePoint(composition.size().map(f64::from)),
                source.spec.frame.composition.area(),
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let check = source.renderer.check_look_projection(
                &self.device,
                &mut encoder,
                &view.image.linear,
                mapping,
            )?;
            view.image.set_look_check(Some(check));
            submissions.submit(&self.queue, [encoder.finish()]);
            let repaint = self.repaint.clone();
            view.image.begin_look_check_tracked(submissions, move || {
                if let Some(context) = repaint.get() {
                    context.request_repaint();
                }
            });
        }
        while view.in_flight() {
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|error| Error::Gpu(error.to_string()))?;
            if !self.active(job) {
                return Ok(None);
            }
        }
        view.image
            .look_result()
            .expect("completed thumbnail look check")?;
        metrics.image_seconds = tick.elapsed().as_secs_f64();
        Ok(self.active(job).then(|| {
            Arc::new(Product {
                image: view.image.clone(),
                positive: Some((source, view)),
            })
        }))
    }

    fn finish(&self, mut work: Work) -> Outcome {
        let mut proper = false;
        let job = &work.job;
        let image = (|| {
            let mut proxy = match work.input? {
                Input::Positive => {
                    proper = true;
                    return self.positive(job, None, &mut work.metrics);
                }
                Input::Retired => return Ok(None),
                Input::Proxy(proxy) => proxy,
                Input::Source(source) => {
                    let mut scan = Scan::from_source(source);
                    let tick = std::time::Instant::now();
                    document::verify_scan(&job.source, &scan)?;
                    work.metrics.verification_seconds = tick.elapsed().as_secs_f64();
                    if job.cancelled.is_cancelled() {
                        return Ok(None);
                    }
                    if let Some(key) = job.key.source.store_key_at(&job.path)
                        && let Some(canonical) =
                            job.store.publish_source(key, scan.source().clone())?
                    {
                        scan = Scan::from_source(canonical);
                    }
                    let tick = std::time::Instant::now();
                    let proxy = match job
                        .key
                        .source
                        .store_key_at(&job.path)
                        .and_then(|key| job.store.get_proxy(&key))
                    {
                        Some(proxy) => proxy,
                        None => Arc::new(Proxy::from_scan(&scan)?),
                    };
                    work.metrics.proxy_seconds = tick.elapsed().as_secs_f64();
                    proxy
                }
                Input::Loading(_) => unreachable!("source acquisition precedes proxy work"),
            };

            // Proper CPU/disk completion is document demand, not the disposable GPU ticket.
            if job.cancelled.is_cancelled() {
                return Ok(None);
            }
            if let Some(key) = job.key.source.store_key_at(&job.path)
                && let Some(canonical) = job.store.publish_proxy(key, proxy.clone())?
            {
                proxy = canonical;
            }
            if let Some(disk) = &self.disk
                && let Err(error) = disk.store_if_missing(&job.key.source, &proxy)
            {
                log::warn!(
                    source_id = job.source.id.0,
                    error = error.to_string().as_str();
                    "thumbnail_disk_cache_write_failed"
                );
            }
            proper = true;
            if !job.render || !self.active(job) {
                return Ok(None);
            }
            if matches!(&job.key.recipe, Ok(Some(_))) {
                let recipe = job.key.recipe.as_ref().unwrap().as_ref().unwrap();
                let prepared = crate::photo_preparation::proxy(
                    Some(&job.store),
                    &job.reference,
                    &job.path,
                    &job.key.source.file,
                    &job.source.development,
                    recipe.neutral.input,
                    &proxy,
                );
                return self.positive(job, Some(&prepared), &mut work.metrics);
            }
            make_image(
                &self.device,
                &self.queue,
                &self.renderers,
                job,
                &proxy,
                &|| self.active(job),
                &mut work.metrics,
            )
        })()
        .map_err(|error: Error| error.to_string());
        log::debug!(
            source_id = job.source.id.0,
            target:? = job.key.target,
            source_cache_hit = work.cache_hit,
            disk_cache_hit = work.metrics.disk_hit,
            proper,
            render = job.render,
            cancelled = !self.active(job),
            failed = image.is_err(),
            seconds = work.started.elapsed().as_secs_f64(),
            phases:? = work.metrics;
            "thumbnail"
        );
        let mut key = job.key.clone();
        if let Ok(Some(recipe)) = &mut key.recipe {
            Arc::make_mut(recipe).neutral.reference = job.reference.get().cloned();
        }
        Outcome {
            ticket: job.ticket,
            key,
            image,
            proper,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn worker(
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderers: Arc<crate::renderer::RendererCache>,
    commands: mpsc::Receiver<workflow::Submission>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    disk: Option<disk::DiskCache>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    developments: source_loading::SourceLoader,
) {
    let worker = Worker {
        device,
        queue,
        renderers,
        generation,
        disk,
        repaint,
        developments,
        kernels: Default::default(),
    };
    let mut pending: Vec<(Work, futures_channel::oneshot::Sender<Outcome>)> = Vec::new();
    let mut connected = true;
    loop {
        // Do not block thumbnail admission behind an eager native waiter. Newly visible
        // misses enter the same native queue while older CPU/disk demand stays owned.
        while let Ok(submission) = commands.try_recv() {
            pending.push((worker.begin(submission.job), submission.reply));
        }
        for (work, _) in &mut pending {
            if work.job.cancelled.is_cancelled() {
                work.input = Ok(Input::Retired);
                continue;
            }
            if let Ok(Input::Loading(load)) = &mut work.input {
                let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
                if let std::task::Poll::Ready(result) = load.as_mut().poll(&mut cx) {
                    work.metrics.development_seconds =
                        work.started.elapsed().as_secs_f64() - work.metrics.disk_seconds;
                    work.input = result.map(Input::Source);
                }
            }
        }
        if let Some(index) = pending
            .iter()
            .enumerate()
            .filter(|(_, (work, _))| !matches!(work.input, Ok(Input::Loading(_))))
            .min_by_key(|(_, (work, _))| work.job.priority)
            .map(|(index, _)| index)
        {
            let (work, reply) = pending.remove(index);
            let outcome = worker.finish(work);
            // Retiring one document's waiter does not stop the shared resource worker.
            let _ = reply.send(outcome);
            continue;
        }
        if !connected {
            break;
        }
        let received = if pending.is_empty() {
            commands
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            commands.recv_timeout(Duration::from_millis(20))
        };
        match received {
            Ok(submission) => pending.push((worker.begin(submission.job), submission.reply)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => connected = false,
        }
    }
    if let Some(context) = worker.repaint.get() {
        context.request_repaint();
    }
}
