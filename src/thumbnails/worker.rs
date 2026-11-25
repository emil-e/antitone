//! Decode and render work with visible-before-eager GPU submission.
use super::{ImageKey, Product, Proxy, disk};
#[cfg(test)]
use crate::project::SourceId;
use crate::{
    Error, evaluation, geometry::Composition, gpu::submission::Queue,
    photographic::stages::look::Looks, source as source_loading,
};
use std::sync::Arc;

pub(crate) struct Job {
    pub(crate) cancelled: crate::cancellation::Cancellation,
    pub(crate) reference: Arc<std::sync::OnceLock<crate::exposure_reference::ReferenceExposure>>,
    pub(crate) ticket: u64,
    pub(crate) generation: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) key: ImageKey,
    pub(crate) physical: crate::source::Input,
    pub(crate) correction: crate::photographic::stages::flat_field::resources::Resources,
    pub(crate) render: bool,
    pub(crate) priority: source_loading::Priority,
    pub(crate) previous: Option<Arc<Product>>,
}

pub(crate) struct Outcome {
    pub(crate) proxy: Option<Arc<Proxy>>,
    pub(crate) ticket: u64,
    pub(crate) key: ImageKey,
    pub(crate) image: Result<Option<Arc<Product>>, String>,
    pub(crate) proper: bool,
}

#[derive(Clone)]
pub(crate) struct ThumbnailGpu {
    pub(crate) evaluation: evaluation::Context,
    pub(crate) queue: Queue,
    pub(crate) looks: Arc<Looks>,
    pub(crate) display: Arc<crate::editor::display::Warmup>,
}

#[derive(Default, Debug)]
pub(crate) struct ThumbnailMetrics {
    pub(crate) disk_hit: bool,
    pub(crate) camera_reused: bool,
    pub(crate) calculation_seconds: f64,
    pub(crate) disk_seconds: f64,
    pub(crate) development_seconds: f64,
    pub(crate) renderer_seconds: f64,
    pub(crate) image_seconds: f64,
}

fn make_image(
    gpu: &ThumbnailGpu,
    job: &Job,
    proxy: &Proxy,
    active: &impl Fn() -> bool,
    metrics: &mut ThumbnailMetrics,
) -> Result<Option<Arc<Product>>, Error> {
    if !active() {
        return Ok(None);
    }
    let tick = std::time::Instant::now();
    let targets = gpu.display.ready()?;
    metrics.renderer_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    let image = pollster::block_on(proxy.image(
        gpu.evaluation.clone(),
        gpu.looks.clone(),
        targets.get(job.key.target),
        job.key.source.source.active_area,
    ))?;
    metrics.image_seconds = tick.elapsed().as_secs_f64();
    Ok(active().then(|| Product::camera(image)))
}

#[cfg(test)]
pub(crate) struct Submission {
    pub job: Job,
    pub reply: futures_channel::oneshot::Sender<Outcome>,
}

#[derive(Clone)]
pub(crate) enum Services {
    Production(Arc<Worker>),
    #[cfg(test)]
    Controlled(std::sync::mpsc::Sender<Submission>),
}

#[derive(Debug)]
pub(crate) struct Stopped;

impl Services {
    pub async fn process(
        self,
        job: Job,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
    ) -> Result<Outcome, Stopped> {
        match self {
            Self::Production(worker) => worker.process(job, sources.ok_or(Stopped)?).await,
            #[cfg(test)]
            Self::Controlled(commands) => {
                let (reply, result) = futures_channel::oneshot::channel();
                commands
                    .send(Submission { job, reply })
                    .map_err(|_| Stopped)?;
                result.await.map_err(|_| Stopped)
            }
        }
    }
}

enum Input {
    Proxy(Arc<Proxy>),
    Missing,
    Positive,
    Retired,
}

struct Work {
    job: Job,
    input: Result<Input, Error>,
    started: std::time::Instant,
    cache_hit: bool,
    metrics: ThumbnailMetrics,
    prepared: Option<crate::scan::CameraProxy>,
}

pub(crate) struct Worker {
    ready: std::sync::Mutex<ReadyWork>,
    #[cfg(test)]
    pub(crate) observe: Option<Arc<dyn Fn(SourceId, bool) + Send + Sync>>,
    gpu: ThumbnailGpu,
    disk: Option<disk::DiskCache>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    kernels: std::sync::OnceLock<Result<Arc<crate::viewer::view::ViewKernels>, Error>>,
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
                            source_id = job.key.source.id.0,
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
            Ok(Input::Missing)
        })();
        Work {
            job,
            input,
            started,
            cache_hit,
            metrics,
            prepared: None,
        }
    }

    pub(crate) fn positive(
        &self,
        job: &Job,
        recipe: &Arc<crate::image_recipe::ImageRecipe>,
        proxy: Option<&crate::scan::CameraProxy>,
        metrics: &mut ThumbnailMetrics,
    ) -> Result<Option<Arc<Product>>, Error> {
        use crate::photographic::graph::view_source::SourceRequest;
        use crate::viewer::view::RenderedView;
        use crate::viewer::view::ViewImage;
        use crate::viewer::view::ViewKernels;
        if !self.active(job) {
            return Ok(None);
        }
        let mut recipe = recipe.clone();
        Arc::make_mut(&mut recipe).neutral.source_development =
            job.physical.development().expect("photo development");
        let tick = std::time::Instant::now();
        let targets = self.gpu.display.ready()?;
        let device = self.gpu.evaluation.device();
        let transform = targets.get(job.key.target);
        metrics.renderer_seconds = tick.elapsed().as_secs_f64();
        let previous = job
            .previous
            .as_ref()
            .and_then(|product| product.positive.as_ref());
        let tick = std::time::Instant::now();
        let capture =
            crate::photographic::graph::request::PhotographRequest::Recipe(recipe.clone());
        let view = if let Some(previous) = previous {
            let source = previous.product().expect("photographic thumbnail product");
            RenderedView::render_from_retained(
                &self.gpu.queue,
                source.clone(),
                capture,
                job.correction.gpu()?,
                &self.gpu.looks,
                transform,
                &|| !self.active(job),
            )?
        } else {
            let proxy =
                proxy.ok_or_else(|| Error::Gpu("thumbnail requires a camera proxy".into()))?;
            let tick = std::time::Instant::now();
            let mut frame = crate::frame_raster::FrameRaster::full(Composition::new(
                job.key.source.source.active_area.into(),
                Default::default(),
                Default::default(),
            )?)?;
            frame.output = proxy.dimensions();
            let spec = SourceRequest::for_frame(frame)?;
            let kernels = self
                .kernels
                .get_or_init(|| pollster::block_on(ViewKernels::new(device)))
                .clone()?;
            let source = ViewImage::camera_preview(
                self.gpu.evaluation.clone(),
                &self.gpu.queue,
                &self.gpu.looks,
                transform,
                kernels,
                proxy,
                job.correction.gpu()?,
                spec,
                &capture,
                &|| !self.active(job),
            )?;
            metrics.calculation_seconds = tick.elapsed().as_secs_f64();
            RenderedView::from_image(device, source, capture, job.key.target)?
        };
        metrics.camera_reused = previous.is_some();
        view.ensure_submitted(
            device,
            &self.gpu.queue,
            self.repaint.get().unwrap_or(&egui::Context::default()),
        );
        while view.in_flight() {
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|error| Error::Gpu(error.to_string()))?;
            if !self.active(job) {
                return Ok(None);
            }
        }
        metrics.image_seconds = tick.elapsed().as_secs_f64();
        Ok(self.active(job).then(|| {
            Arc::new(Product {
                image: view.image.clone(),
                positive: Some(view),
            })
        }))
    }

    fn finish(&self, mut work: Work) -> Completion {
        let mut proper = false;
        let mut produced = None;
        let mut captured = None;
        let job = &work.job;
        let image = (|| {
            let proxy = match work.input? {
                Input::Positive => {
                    proper = true;
                    let recipe = job
                        .key
                        .recipe
                        .as_ref()
                        .map_err(|error| Error::Gpu(error.clone()))?
                        .as_ref()
                        .ok_or(Error::Gpu("positive thumbnail requires a recipe".into()))?;
                    captured = Some(crate::photo_preparation::ready_image(
                        recipe,
                        &job.reference,
                    ));
                    return self.positive(job, captured.as_ref().unwrap(), None, &mut work.metrics);
                }
                Input::Retired => return Ok(None),
                Input::Proxy(proxy) => proxy,
                Input::Missing => unreachable!("source acquisition precedes proxy work"),
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
                    source_id = job.key.source.id.0,
                    error = error.to_string().as_str();
                    "thumbnail_disk_cache_write_failed"
                );
            }
            proper = true;
            if !job.render || !self.active(job) {
                return Ok(None);
            }
            if matches!(&job.key.recipe, Ok(Some(_))) {
                let prepared = work.prepared.as_ref().ok_or(Error::UnusableSample(
                    "positive proxy has not been acquired",
                ))?;
                let recipe = job.key.recipe.as_ref().unwrap().as_ref().unwrap();
                // Acquisition resolved the reference; the recipe's recorded value still wins.
                let (prepared, recipe) = crate::photo_preparation::proxy_image(
                    recipe,
                    &job.reference,
                    &prepared.data,
                    Ok(None),
                );
                captured = Some(recipe);
                return self.positive(
                    job,
                    captured.as_ref().unwrap(),
                    Some(&prepared),
                    &mut work.metrics,
                );
            }
            make_image(
                &self.gpu,
                job,
                &proxy,
                &|| self.active(job),
                &mut work.metrics,
            )
        })()
        .map_err(|error: Error| error.to_string());
        log::debug!(
            source_id = job.key.source.id.0,
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
        if let Some(recipe) = captured {
            key.recipe = Ok(Some(recipe));
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
struct ReadyWork {
    pending: Vec<(Work, futures_channel::oneshot::Sender<Completion>)>,
    running: bool,
}

impl Worker {
    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        self.ready.lock().unwrap().pending.len()
    }

    pub(crate) fn new(
        gpu: ThumbnailGpu,
        disk: Option<disk::DiskCache>,
        repaint: Arc<std::sync::OnceLock<egui::Context>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            gpu,
            disk,
            repaint,
            kernels: Default::default(),
            ready: Default::default(),
            #[cfg(test)]
            observe: None,
        })
    }

    pub(crate) async fn process(
        self: Arc<Self>,
        mut job: Job,
        sources: Arc<dyn crate::source::SourceProvider>,
    ) -> Result<Outcome, Stopped> {
        let cancelled = job.cancelled.clone();
        job.key.source = job
            .key
            .source
            .with_development(job.physical.development().expect("photo development"));
        let cached = job
            .key
            .source
            .store_key_at(job.physical.path())
            .and_then(|key| sources.available_proxy(&key));
        let worker = self.clone();
        let mut work = crate::background::run(move |_| worker.begin(job, cached))
            .await
            .map_err(|_| Stopped)?;
        let positive = work.job.render && matches!(&work.job.key.recipe, Ok(Some(_)));
        if positive
            && matches!(
                &work.input,
                Ok(Input::Missing | Input::Proxy(_) | Input::Positive)
            )
        {
            let correction = futures_lite::future::or(
                async {
                    work.job.cancelled.wait().await;
                    Ok(None)
                },
                async { work.job.correction.clone().acquire().await.map(Some) },
            )
            .await;
            match correction {
                Ok(Some(correction)) => work.job.correction = correction.into(),
                Ok(None) => work.input = Ok(Input::Retired),
                Err(error) => work.input = Err(Error::Gpu(error)),
            }
        }
        if positive && matches!(&work.input, Ok(Input::Missing | Input::Proxy(_))) {
            let cached = match &work.input {
                Ok(Input::Proxy(proxy)) => Some(proxy.clone()),
                _ => None,
            };
            let prepared = futures_lite::future::or(
                async {
                    work.job.cancelled.wait().await;
                    Ok(None)
                },
                async {
                    work.job
                        .physical
                        .positive_proxy(sources.as_ref(), work.job.priority, cached)
                        .await
                        .map(Some)
                },
            )
            .await;
            work.input = match prepared {
                Ok(Some(prepared)) => {
                    let pixels = prepared.data.clone();
                    work.prepared = Some(prepared);
                    Ok(Input::Proxy(pixels))
                }
                Ok(None) => Ok(Input::Retired),
                Err(error) => Err(error),
            };
        } else if matches!(&work.input, Ok(Input::Missing)) {
            work.input = futures_lite::future::or(
                async {
                    work.job.cancelled.wait().await;
                    Ok(Input::Retired)
                },
                async {
                    work.job
                        .physical
                        .proxy(sources.as_ref(), work.job.priority)
                        .await
                        .map(Input::Proxy)
                },
            )
            .await;
        }
        work.metrics.development_seconds =
            work.started.elapsed().as_secs_f64() - work.metrics.disk_seconds;
        if let Ok(record) = work.job.physical.record() {
            work.job.key.source.source = record.development.for_runtime();
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
        let completion = result.await.map_err(|_| Stopped)?;
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
                let id = work.job.key.source.id;
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
