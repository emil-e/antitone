use super::*;

#[cfg(test)]
mod tests;

enum Input {
    Proxy(Arc<Proxy>),
    Missing(Box<crate::source::SourceKey>),
    Positive,
    Retired,
}

struct Work {
    job: Job,
    input: Result<Input, Error>,
    started: std::time::Instant,
    cache_hit: bool,
    metrics: ThumbnailMetrics,
    preview: Option<crate::source::PreviewResult>,
}

pub(super) struct Worker {
    ready: std::sync::Mutex<Ready>,
    #[cfg(test)]
    observe: Option<Arc<dyn Fn(SourceId, bool) + Send + Sync>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderers: Arc<crate::renderer::RendererCache>,
    disk: Option<disk::DiskCache>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    kernels: std::sync::OnceLock<Result<Arc<crate::gpu::view::ViewKernels>, Error>>,
}

impl Worker {
    fn active(&self, job: &Job) -> bool {
        job.generation.load(std::sync::atomic::Ordering::Relaxed) == job.ticket
            && !job.cancelled.is_cancelled()
    }

    fn begin(&self, job: Job, cached: Option<Arc<Proxy>>) -> Work {
        let started = std::time::Instant::now();
        let cache_hit = cached.is_some();
        let mut metrics = ThumbnailMetrics::default();
        let input = (|| {
            if job.cancelled.is_cancelled() {
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
            if job.cancelled.is_cancelled() {
                return Ok(Input::Retired);
            }
            let key = crate::source::SourceKey::for_load(
                &job.path,
                job.key.source.file.clone(),
                &job.source.development,
            );
            Ok(Input::Missing(Box::new(key)))
        })();
        Work {
            job,
            input,
            started,
            cache_hit,
            metrics,
            preview: None,
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
                source.spec.frame().composition.area(),
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

    fn finish(&self, mut work: Work) -> Completion {
        let mut proper = false;
        let mut produced = None;
        let job = &work.job;
        let image = (|| {
            let proxy = match work.input? {
                Input::Positive => {
                    proper = true;
                    return self.positive(job, None, &mut work.metrics);
                }
                Input::Retired => return Ok(None),
                Input::Proxy(proxy) => proxy,
                Input::Missing(_) => unreachable!("source acquisition precedes proxy work"),
            };

            // Proper CPU/disk completion is document demand, not the disposable GPU ticket.
            if job.cancelled.is_cancelled() {
                return Ok(None);
            }
            produced = Some(proxy.clone());
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
                    &job.reference,
                    &job.source.development,
                    recipe.neutral.input,
                    &proxy,
                    work.preview.unwrap_or(Ok(None)),
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
        Completion {
            outcome: Outcome {
                proxy: None,
                ticket: job.ticket,
                key,
                image,
                proper,
            },
            proxy: produced,
        }
    }
}

struct Completion {
    outcome: Outcome,
    proxy: Option<Arc<Proxy>>,
}

#[derive(Default)]
struct Ready {
    pending: Vec<(Work, futures_channel::oneshot::Sender<Completion>)>,
    running: bool,
}

impl Worker {
    pub(super) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        disk: Option<disk::DiskCache>,
        repaint: Arc<std::sync::OnceLock<egui::Context>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            device,
            queue,
            renderers,
            disk,
            repaint,
            kernels: Default::default(),
            ready: Default::default(),
            #[cfg(test)]
            observe: None,
        })
    }

    pub(super) async fn process(
        self: Arc<Self>,
        job: Job,
        sources: Arc<dyn crate::source::SourceProvider>,
    ) -> Result<Outcome, workflow::Stopped> {
        let cancelled = job.cancelled.clone();
        let cached = job
            .key
            .source
            .store_key_at(&job.path)
            .and_then(|key| sources.available_proxy(&key));
        let worker = self.clone();
        let mut work = crate::background::run(move || worker.begin(job, cached))
            .await
            .map_err(|_| workflow::Stopped)?;
        if let Ok(Input::Missing(key)) = &work.input {
            let key = (**key).clone();
            let cancelled = work.job.cancelled.clone();
            work.input = futures_lite::future::or(
                async {
                    cancelled.wait().await;
                    Ok(Input::Retired)
                },
                async {
                    sources
                        .load_proxy(key, work.job.priority)
                        .await
                        .map(Input::Proxy)
                },
            )
            .await;
            work.metrics.development_seconds =
                work.started.elapsed().as_secs_f64() - work.metrics.disk_seconds;
        }
        if work.job.render
            && !work.job.cancelled.is_cancelled()
            && work.job.reference.get().is_none()
            && matches!(&work.job.key.recipe, Ok(Some(recipe)) if !recipe.neutral.input.is_negative())
        {
            let path = work.job.path.clone();
            let development = work.job.source.development.clone();
            let key = crate::source::SourceKey::for_load(
                &path,
                work.job.key.source.file.clone(),
                &development,
            );
            let load = sources.load_preview(key);
            work.preview = futures_lite::future::or(
                async {
                    work.job.cancelled.wait().await;
                    None
                },
                async { Some(load.await) },
            )
            .await;
        }
        let (reply, result) = futures_channel::oneshot::channel();
        let start = {
            let mut ready = self.ready.lock().unwrap();
            ready.pending.push((work, reply));
            let start = !ready.running;
            ready.running = true;
            start
        };
        if start {
            // One pool job drains ready CPU/GPU work; native/preview waits stay in futures.
            blocking::unblock(move || self.drain()).detach();
        }
        let completion = result.await.map_err(|_| workflow::Stopped)?;
        let mut outcome = completion.outcome;
        if !cancelled.is_cancelled() {
            outcome.proxy = completion.proxy;
        }
        Ok(outcome)
    }

    fn drain(&self) {
        loop {
            let next = {
                let mut ready = self.ready.lock().unwrap();
                let index = ready
                    .pending
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (work, _))| work.job.priority)
                    .map(|(index, _)| index);
                match index {
                    Some(index) => Some(ready.pending.remove(index)),
                    None => {
                        ready.running = false;
                        None
                    }
                }
            };
            let Some((work, reply)) = next else {
                return;
            };
            if let Ok(outcome) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #[cfg(test)]
                let id = work.job.source.id;
                #[cfg(test)]
                if let Some(observe) = &self.observe {
                    observe(id, true);
                }
                let outcome = self.finish(work);
                #[cfg(test)]
                if let Some(observe) = &self.observe {
                    observe(id, false);
                }
                outcome
            })) {
                let _ = reply.send(outcome);
            }
        }
    }
}
