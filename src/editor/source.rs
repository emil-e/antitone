//! Explicit source acquisition and preparation. Each consumer leases immutable
//! development and carries its own correction; no selected-source slot is needed.
use super::{Correction, Document, Editor, Ready, document, source_loading, thumbnails};
use crate::{
    Error,
    project::Source,
    scan::{DevelopedSource, Scan},
    source::{SourceCache, SourceKey, SourceLoader, SourceProvider, SourceStore},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
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
                .require_source(&document::metadata_development(
                    &self.source.metadata(),
                    self.source.fixed_wb(),
                )?)?;
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
    pub(super) fn provider(&self, loader: SourceLoader) -> SourceCache<SourceLoader> {
        SourceCache::new(loader, self.store.clone()).with_parent(self.parent.clone())
    }

    pub fn new(path: &Path, expected: Option<&Source>, document: &Document) -> Self {
        let file = document.file_key(path);
        let key = expected
            .and_then(|source| SourceKey::from_revision(path, file.clone(), &source.development));
        Self {
            store: document.cache().clone(),
            parent: document.parent_cache().cloned(),
            file,
            key,
        }
    }
}

pub(super) struct Request {
    pub input: crate::processing::InputSettings,
    pub path: PathBuf,
    pub wb: Option<[f64; 4]>,
    pub expected: Option<Source>,
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
    camera_proxy: Option<Arc<crate::scan::CameraProxy>>,
}

#[derive(Clone)]
pub(super) struct Services {
    pub developments: source_loading::SourceLoader,
    pub renderers: Arc<crate::renderer::RendererCache>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
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
        crate::background::run(move || {
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
        let started = std::time::Instant::now();
        let retired = cancelled.clone();

        let needs_header = request.expected.is_none();
        let capture = move || {
            let mut request = request;
            let cache = request.cache.clone();
            if independent {
                request.cache = None;
            }
            request.require_live(&|| retired.load(std::sync::atomic::Ordering::Relaxed))?;
            let key = request.load_key()?;
            Ok::<_, Error>((request, key, cache))

        };
        let captured = if needs_header {
            crate::background::run(capture)
                .await
                .map_err(|_| "Source header worker panicked".to_owned())?
        } else {
            capture()
        };
        let (request, key, cache) = captured.map_err(|error| error.to_string())?;
        let references = cache.as_ref().map_or_else(
            || Arc::new(SourceStore::new(0)), |cache| cache.store.clone(),
        );
        let store = (!independent)
            .then(|| cache.as_ref().map(|cache| cache.store.clone()))
            .flatten();
        let loader = self.developments.clone();
        let load = async {
            match cache {
                Some(cache) => {
                    cache
                        .provider(loader)
                        .load_source(key.clone(), source_loading::Priority::Foreground)
                        .await
                }
                None => {
                    loader
                        .load_source(key.clone(), source_loading::Priority::Foreground)
                        .await
                }
            }
        };
        let source = if let Some(store) = store {
            let changed = store.listen_for_close();
            let close = async {
                if !store.is_closed() {
                    changed.await;
                }
                Err(Error::UnusableSample("document source work retired"))
            };
            futures_lite::future::or(close, load).await
        } else {
            load.await
        }
        .map_err(|error| error.to_string())?;
        let timing = Timing {
            started,
            development_seconds: started.elapsed().as_secs_f64(),
            registration_seconds: 0.,
            verification_seconds: 0.,
        };
        let file = key.file().clone();

        crate::background::run(move || {
            request.accept(
                source,
                file,
                references,
                &|| cancelled.load(std::sync::atomic::Ordering::Relaxed),
                timing,
            )
        })
        .await
        .map_err(|_| "Source preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }
}

struct Timing {
    started: std::time::Instant,
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


    pub(super) fn load_key(&self) -> Result<SourceKey, Error> {
        let record = match &self.expected {
            Some(expected) => {
                expected.grade_snapshot()?;
                if self
                    .wb
                    .is_some_and(|wb| wb != expected.development.decode.fixed_wb)
                {
                    return Err(Error::Project(
                        "source WB differs from requested development".into(),
                    ));
                }
                expected.clone()
            }
            None => document::open_source(&self.path, self.wb)?.0,
        };
        let file = if self.expected.is_none() {
            thumbnails::FileKey::new(&self.path)
        } else {
            self.cache.as_ref().map_or_else(
                || thumbnails::FileKey::unversioned(&self.path),
                |cache| cache.file.clone(),
            )
        };
        Ok(SourceKey::for_load(&self.path, file, &record.development))
    }

    /// Synchronous fixtures drive the same provider, observing their poll-only stop signal.
    #[cfg(test)]
    pub fn load(
        self,
        developments: &source_loading::SourceLoader,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let key = self.load_key()?;
        let file = key.file().clone();
        let started = std::time::Instant::now();
        let loader = developments.clone();
        let mut load = Box::pin(async {
            match &self.cache {
                Some(cache) => {
                    cache
                        .provider(loader)
                        .load_source(key, source_loading::Priority::Foreground)
                        .await
                }
                None => {
                    loader
                        .load_source(key, source_loading::Priority::Foreground)
                        .await
                }
            }
        });
        let source = loop {
            self.require_live(cancelled)?;
            if let Some(result) =
                futures_lite::future::block_on(futures_lite::future::poll_once(&mut load))
            {
                break result?;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        drop(load);
        let timing = Timing {
            started,
            development_seconds: started.elapsed().as_secs_f64(),
            registration_seconds: 0.,
            verification_seconds: 0.,
        };
        let references = self.cache.as_ref().map_or_else(
            || Arc::new(SourceStore::new(0)),
            |cache| cache.store.clone(),
        );
        self.accept(source, file, references, cancelled, timing)
    }

    fn accept(
        self,
        source: Arc<DevelopedSource>,
        file: thumbnails::FileKey,
        references: Arc<SourceStore>,
        cancelled: &dyn Fn() -> bool,
        mut timing: Timing,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let tick = std::time::Instant::now();
        let record = document::source_metadata(&self.path, &source.metadata(), source.fixed_wb())?;
        timing.registration_seconds = tick.elapsed().as_secs_f64();
        if let Some(expected) = &self.expected {
            document::verify_source(expected, &record)?;
        }
        let tick = std::time::Instant::now();
        let mut scan = Scan::from_source(source);
        document::verify_scan(&record, &scan)?;
        timing.verification_seconds = tick.elapsed().as_secs_f64();
        let prepared = references.prepare_source(self.input, scan.source())?;
        let camera_proxy = prepared.proxy;
        scan = Scan::from_source(prepared.source);
        self.require_live(cancelled)?;
        let key = SourceKey::from_revision(&self.path, file.clone(), &record.development);
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
            camera_proxy,
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
            .camera_proxy
            .or_else(|| {
                self.cache
                    .as_ref()
                    .and_then(|cache| self.key.as_ref().and_then(|key| cache.store.get_proxy(key)))
            })
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
            development_seconds,
            registration_seconds,
            verification_seconds,
        } = self.timing;

        log::debug!(
            seconds = started.elapsed().as_secs_f64(),
            registration_seconds,
            development_seconds,
            verification_seconds,
            proxy_seconds,
            renderer_seconds,
            images_seconds;
            "selected_source"
        );

        Ok(Ready {
            developed: self.context.source,
            file: self.file,
            thumbnail,
            path: self.path,
            area: scan.area,
            analysis_support: None,
            proxy,
            record: self.record,
        })
    }
}
