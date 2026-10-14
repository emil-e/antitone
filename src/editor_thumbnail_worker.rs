use super::*;
use crate::scan::DevelopedSource;

enum Input {
    Proxy(Arc<Proxy>),
    Source(Arc<DevelopedSource>),
    Native(development::Ticket),
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
    developments: development::Scheduler,
}
impl Worker {
    fn active(&self, job: &Job) -> bool {
        self.generation.load(std::sync::atomic::Ordering::Relaxed) == job.ticket
            && !job.store.is_closed()
    }
    fn begin(&self, job: Job) -> Work {
        let started = std::time::Instant::now();
        let native_key = job.key.source.store_key_at(&job.path);
        let cached = native_key.as_ref().and_then(|key| job.store.get_proxy(key));
        let cache_hit = cached.is_some();
        let mut metrics = ThumbnailMetrics::default();
        let input = (|| {
            if !self.active(&job) || FileKey::new(&job.path) != job.key.source.file {
                return Ok(Input::Retired);
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
                        eprintln!(
                            "thumbnail disk_cache_miss source_id={} reason={error}",
                            job.source.id.0
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
            if let Some(source) = native_key.as_ref().and_then(|key| job.store.get(key)) {
                metrics.native_hit = true;
                return Ok(Input::Source(source));
            }
            self.developments
                .submit(
                    development::Request {
                        path: job.path.clone(),
                        wb: Some(job.key.source.development.fixed_wb),
                        expected: Some(job.source.clone()),
                        basis: None,
                        file: job.key.source.file.clone(),
                        key: native_key,
                        store: Some(job.store.clone()),
                        parent: None,
                        retain: true,
                    },
                    job.priority,
                )
                .map(Input::Native)
        })();
        Work {
            job,
            input,
            started,
            cache_hit,
            metrics,
        }
    }
    fn finish(&self, mut work: Work) -> Outcome {
        let mut proper = false;
        let job = &work.job;
        let image = (|| {
            let mut proxy = match work.input? {
                Input::Retired => return Ok(None),
                Input::Proxy(proxy) => proxy,
                Input::Source(source) => {
                    let mut scan = Scan::from_source(source);
                    let tick = std::time::Instant::now();
                    document::verify_scan(&job.source, &scan)?;
                    work.metrics.verification_seconds = tick.elapsed().as_secs_f64();
                    if job.store.is_closed() || FileKey::new(&job.path) != job.key.source.file {
                        return Ok(None);
                    }
                    if let Some(key) = job.key.source.store_key_at(&job.path)
                        && let Some(canonical) =
                            job.store.publish_source(key, scan.source().clone())?
                    {
                        scan = Scan::from_source(canonical);
                    }
                    let tick = std::time::Instant::now();
                    let proxy = Arc::new(Proxy::from_scan(&scan)?);
                    work.metrics.proxy_seconds = tick.elapsed().as_secs_f64();
                    proxy
                }
                Input::Native(_) => unreachable!("native completion precedes proxy work"),
            };
            // Proper CPU/disk completion is document demand, not the disposable GPU ticket.
            if job.store.is_closed() || FileKey::new(&job.path) != job.key.source.file {
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
                eprintln!(
                    "thumbnail disk_cache_write_failed source_id={} reason={error}",
                    job.source.id.0
                );
            }
            proper = true;
            if !job.render || !self.active(job) {
                return Ok(None);
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
        println!(
            "thumbnail source_id={} target={:?} source_cache_hit={} disk_cache_hit={} proper={proper} render={} cancelled={} failed={} seconds={:.6} phases={:?}",
            job.source.id.0,
            job.key.target,
            work.cache_hit,
            work.metrics.disk_hit,
            job.render,
            !self.active(job),
            image.is_err(),
            work.started.elapsed().as_secs_f64(),
            work.metrics
        );
        Outcome {
            ticket: job.ticket,
            key: job.key.clone(),
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
    commands: mpsc::Receiver<Job>,
    messages: mpsc::Sender<Outcome>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    disk: Option<disk::DiskCache>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    developments: development::Scheduler,
) {
    let worker = Worker {
        device,
        queue,
        renderers,
        generation,
        disk,
        repaint,
        developments,
    };
    let mut pending: Vec<Work> = Vec::new();
    let mut connected = true;
    loop {
        // Do not block thumbnail admission behind an eager native waiter. Newly visible
        // misses enter the same native queue while older CPU/disk demand stays owned.
        while let Ok(job) = commands.try_recv() {
            pending.push(worker.begin(job));
        }
        for work in &mut pending {
            if let Ok(Input::Native(ticket)) = &work.input {
                match ticket.result() {
                    Ok(None) => {}
                    result => {
                        work.metrics.development_seconds =
                            work.started.elapsed().as_secs_f64() - work.metrics.disk_seconds;
                        work.input = result.map(|source| Input::Source(source.unwrap()));
                    }
                }
            }
        }
        if let Some(index) = pending
            .iter()
            .enumerate()
            .filter(|(_, work)| !matches!(work.input, Ok(Input::Native(_))))
            .min_by_key(|(_, work)| work.job.priority)
            .map(|(index, _)| index)
        {
            let outcome = worker.finish(pending.remove(index));
            if messages.send(outcome).is_err() {
                break;
            }
            if let Some(context) = worker.repaint.get() {
                context.request_repaint();
            }
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
            Ok(job) => pending.push(worker.begin(job)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => connected = false,
        }
    }
    if let Some(context) = worker.repaint.get() {
        context.request_repaint();
    }
}
