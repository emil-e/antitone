//! Explicit source acquisition and preparation. Each consumer leases immutable
//! development and carries its own correction; no selected-source slot is needed.
use super::{Correction, Document, Editor, Ready, development, document, thumbnails};
use crate::{
    Error,
    project::{CameraBasis, Development, Source},
    scan::{DevelopedSource, Scan},
    source_store::{SourceKey, SourceStore},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
#[path = "editor_source_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) struct Context {
    pub source: Arc<DevelopedSource>,
    pub correction: Option<Arc<Correction>>,
}

impl Editor {
    pub(super) fn source_context(&self) -> Option<Context> {
        Some(Context {
            source: self.document.visit.ready.as_ref()?.developed.clone(),
            correction: if self.flat_edit.is_some() {
                None
            } else {
                self.correction.clone()
            },
        })
    }
}

impl Context {
    pub fn scan(&self) -> Result<Scan, Error> {
        if let Some(correction) = &self.correction {
            correction
                .settings
                .require_source(&document::metadata_development(&self.source.metadata())?)?;
        }
        let mut scan = Scan::from_source(self.source.clone());
        scan.set_flat_field(
            self.correction
                .as_ref()
                .map(|field| field.settings.map.clone()),
        )?;
        Ok(scan)
    }
}

#[derive(Clone)]
pub(super) struct Cache {
    pub store: Arc<SourceStore>,
    pub parent: Option<Arc<SourceStore>>,
    pub file: thumbnails::FileKey,
    pub key: Option<SourceKey>,
}

impl Cache {
    pub fn new(
        path: &Path,
        expected: Option<&Source>,
        wb: Option<[f64; 4]>,
        document: &Document,
    ) -> Self {
        let file = thumbnails::FileKey::new(path);
        let key = expected.zip(wb).and_then(|(source, wb)| {
            SourceKey::from_revision(
                path,
                file.clone(),
                &source.development,
                &Development::for_source(&source.development, wb),
            )
        });
        Self {
            store: document.cache().clone(),
            parent: document.parent_cache().cloned(),
            file,
            key,
        }
    }
}

pub(super) struct Request {
    pub path: PathBuf,
    pub wb: Option<[f64; 4]>,
    pub expected: Option<Source>,
    pub basis: Option<CameraBasis>,
    pub cache: Option<Cache>,
    pub correction: Option<Arc<Correction>>,
}

pub(super) struct Developed {
    pub context: Context,
    pub record: Source,
    file: thumbnails::FileKey,
    path: PathBuf,
    cache: Option<Cache>,
    key: Option<SourceKey>,
    timing: Timing,
}

#[derive(Clone)]
pub(super) struct Services {
    pub developments: development::Scheduler,
    pub jobs: crate::blocking_jobs::BlockingJobs,
    pub renderers: Arc<crate::renderer::RendererCache>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

struct Acquisition {
    request: Request,
    file: thumbnails::FileKey,
    supply: Supply,
    timing: Timing,
}

enum Supply {
    Retained(Arc<DevelopedSource>),
    Native(development::Ticket),
}

impl Services {
    pub async fn prepare(
        self,
        request: Request,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Ready, String> {
        let developed = self
            .clone()
            .develop(request, cancelled.clone(), false)
            .await?;
        self.jobs
            .run(move || {
                developed.prepare_gpu(&self.renderers, &self.device, &self.queue, &|| {
                    cancelled.load(std::sync::atomic::Ordering::Relaxed)
                })
            })
            .await
            .map_err(|_| "Source preparation worker panicked".to_owned())?
            .map_err(|error| error.to_string())
    }

    /// Export demand owns its lifetime; document caches supply optional reuse only.
    pub async fn acquire(
        self,
        request: Request,
        cancelled: crate::cancellation::Cancellation,
    ) -> Result<Context, String> {
        self.develop(request, cancelled.flag(), true)
            .await
            .map(|value| value.context)
    }

    async fn develop(
        self,
        request: Request,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        independent: bool,
    ) -> Result<Developed, String> {
        let scheduler = self.developments.clone();
        let retired = cancelled.clone();
        let acquisition = self
            .jobs
            .run_admitted(move |admission| {
                request.begin(&scheduler, admission, independent, &|| {
                    retired.load(std::sync::atomic::Ordering::Relaxed)
                })
            })
            .await
            .map_err(|_| "Source submission worker panicked".to_owned())?
            .map_err(|error| error.to_string())?;
        let Acquisition {
            request,
            file,
            supply,
            mut timing,
        } = acquisition;
        let source = match supply {
            Supply::Retained(source) => source,
            Supply::Native(ticket) => ticket.await.map_err(|error| error.to_string())?,
        };
        timing.development_seconds = timing.started.elapsed().as_secs_f64();
        self.jobs
            .run(move || {
                let retired = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
                request.accept(source, file, &retired, timing)
            })
            .await
            .map_err(|_| "Source preparation worker panicked".to_owned())?
            .map_err(|error| error.to_string())
    }
}

struct Timing {
    started: std::time::Instant,
    native_hit: bool,
    development_seconds: f64,
    registration_seconds: f64,
    verification_seconds: f64,
}

impl Request {
    fn require_live(&self, cancelled: &dyn Fn() -> bool) -> Result<(), Error> {
        if cancelled()
            || self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.store.is_closed())
        {
            return Err(Error::UnusableSample("document source work retired"));
        }
        Ok(())
    }

    fn demand(&self) -> Result<development::Request, Error> {
        if let Some(expected) = &self.expected {
            expected.grade_snapshot()?;
        }
        Ok(development::Request {
            path: self.path.clone(),
            wb: self.wb,
            expected: self.expected.clone(),
            basis: self.basis.clone(),
            file: self.cache.as_ref().map_or_else(
                || thumbnails::FileKey::new(&self.path),
                |cache| cache.file.clone(),
            ),
            key: self.cache.as_ref().and_then(|cache| cache.key.clone()),
            store: self.cache.as_ref().map(|cache| cache.store.clone()),
            parent: self.cache.as_ref().and_then(|cache| cache.parent.clone()),
            retain: true,
        })
    }

    fn retained(&self) -> Option<Arc<DevelopedSource>> {
        let cache = self.cache.as_ref()?;
        let key = cache.key.as_ref()?;
        cache
            .store
            .get(key)
            .or_else(|| cache.parent.as_ref().and_then(|parent| parent.get(key)))
    }

    fn begin(
        mut self,
        developments: &development::Scheduler,
        admission: crate::blocking_jobs::Admission,
        independent: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Acquisition, Error> {
        let retained = self.retained();
        if independent {
            // The cache may close while native work is queued. Do not use its
            // publication lifetime as authority for an accepted file operation.
            self.cache = None;
        }
        self.require_live(cancelled)?;
        let demand = self.demand()?;
        let file = demand.file.clone();
        let started = std::time::Instant::now();
        let native_hit = retained.is_some();
        let supply = match retained {
            Some(source) => Supply::Retained(source),
            None => Supply::Native(developments.submit_admitted(
                demand,
                development::Priority::Foreground,
                admission,
            )?),
        };
        Ok(Acquisition {
            request: self,
            file,
            supply,
            timing: Timing {
                started,
                native_hit,
                development_seconds: 0.,
                registration_seconds: 0.,
                verification_seconds: 0.,
            },
        })
    }

    /// Synchronous source fixtures exercise acquisition without application state.
    #[cfg(test)]
    pub fn load(
        self,
        developments: &development::Scheduler,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let demand = self.demand()?;
        let file = demand.file.clone();
        let started = std::time::Instant::now();
        let retained = self.retained();
        let native_hit = retained.is_some();
        let tick = std::time::Instant::now();
        let source = match retained {
            Some(source) => source,
            None => developments.load(demand, development::Priority::Foreground, cancelled)?,
        };
        let timing = Timing {
            started,
            native_hit,
            development_seconds: tick.elapsed().as_secs_f64(),
            registration_seconds: 0.,
            verification_seconds: 0.,
        };
        self.accept(source, file, cancelled, timing)
    }

    fn accept(
        self,
        source: Arc<DevelopedSource>,
        file: thumbnails::FileKey,
        cancelled: &dyn Fn() -> bool,
        mut timing: Timing,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let tick = std::time::Instant::now();
        let record = document::source_metadata(&self.path, &source.metadata())?;
        timing.registration_seconds = tick.elapsed().as_secs_f64();
        if let Some(basis) = &self.basis {
            basis.require_source(&record.development)?;
        }
        if let Some(expected) = &self.expected {
            document::verify_source(expected, &record)?;
        }
        let tick = std::time::Instant::now();
        let mut scan = Scan::from_source(source);
        document::verify_scan(&record, &scan)?;
        timing.verification_seconds = tick.elapsed().as_secs_f64();
        self.require_live(cancelled)?;
        // A captured revision can disable cache publication, never be refreshed to
        // relabel pixels developed from an earlier file version.
        let key = SourceKey::from_revision(
            &self.path,
            file.clone(),
            &record.development,
            &scan.source().metadata().development(scan.fixed_wb),
        );
        if let Some(cache) = &self.cache
            && let Some(key) = &key
            && let Some(source) = cache
                .store
                .publish_source(key.clone(), scan.source().clone())?
        {
            scan = Scan::from_source(source);
        }
        if let Some(correction) = &self.correction {
            correction.settings.require_source(&record.development)?;
        }
        let context = Context {
            source: scan.source().clone(),
            correction: self.correction,
        };
        context.scan()?;
        Ok(Developed {
            context,
            record,
            file,
            path: self.path,
            cache: self.cache,
            key,
            timing,
        })
    }
}

impl Developed {
    pub fn prepare_gpu(
        self,
        renderers: &crate::renderer::RendererCache,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Ready, Error> {
        if cancelled()
            || self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.store.is_closed())
        {
            return Err(Error::UnusableSample("document source work retired"));
        }
        let scan = self.context.scan()?;
        let tick = std::time::Instant::now();
        let (width, height, pixels) = scan.preview_source(1024)?;
        let proxy = self
            .cache
            .as_ref()
            .and_then(|cache| self.key.as_ref().and_then(|key| cache.store.get_proxy(key)))
            .map_or_else(|| thumbnails::Proxy::from_scan(&scan).map(Arc::new), Ok)?;
        if let Some(cache) = &self.cache
            && let Some(key) = &self.key
        {
            cache.store.publish_proxy(key.clone(), proxy.clone())?;
        }
        let thumbnail = Some((self.file.clone(), proxy));
        let proxy_seconds = tick.elapsed().as_secs_f64();
        let tick = std::time::Instant::now();
        let renderer = renderers.get(device, queue)?;
        let renderer_seconds = tick.elapsed().as_secs_f64();
        let tick = std::time::Instant::now();
        let a = scan.area;
        let proxy = crate::viewer::GpuProxy::new_continuous(
            renderer,
            device,
            [width, height],
            &pixels,
            scan.camera_to_ap1,
            self.context
                .correction
                .as_ref()
                .map(|correction| correction.gpu.clone()),
            crate::gpu::SourceGeometry::new([a.x, a.y, a.width, a.height], [width, height])?,
        )?;
        let images_seconds = tick.elapsed().as_secs_f64();
        let Timing {
            started,
            native_hit,
            development_seconds,
            registration_seconds,
            verification_seconds,
        } = self.timing;
        println!(
            "selected_source native_cache_hit={native_hit} seconds={:.6} registration_seconds={registration_seconds:.6} development_seconds={development_seconds:.6} verification_seconds={verification_seconds:.6} proxy_seconds={proxy_seconds:.6} renderer_seconds={renderer_seconds:.6} images_seconds={images_seconds:.6}",
            started.elapsed().as_secs_f64()
        );
        Ok(Ready {
            developed: self.context.source,
            file: self.file,
            thumbnail,
            path: self.path,
            area: scan.area,
            analysis_support: None,
            floor: scan.input_floor,
            proxy,
            record: self.record,
            development: scan.source().metadata().development(scan.fixed_wb),
        })
    }
}
