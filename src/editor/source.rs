//! Explicit source acquisition and preparation. Each consumer leases immutable
//! development and carries its own correction; no selected-source slot is needed.
use super::{Correction, Editor, Ready, document, source_loading, thumbnails};
use crate::{
    Error,
    project::Source,
    scan::{DevelopedSource, Scan},
    source::SourceKey,
};
use std::{path::PathBuf, sync::Arc};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct Context {
    pub source: Arc<DevelopedSource>,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
    pub correction: Option<Arc<Correction>>,
}

impl Editor {
    pub(super) fn source_provider(&self) -> Option<Arc<dyn crate::source::SourceProvider>> {
        self.source_services
            .as_ref()
            .map(|services| services.provider(Some(self.document.cache.clone())))
            .or_else(|| {
                self.developments
                    .as_ref()
                    .map(|loader| loader.provider(Some(self.document.cache.clone())))
            })
    }

    pub(super) fn source_context(&self) -> Option<Context> {
        let ready = self.document.visit.canvas.ready.as_ref()?;
        Some(Context {
            source: ready.developed.clone(),
            reference: ready.reference.clone(),
            correction: self.document.correction.clone(),
        })
    }

    pub(super) fn canvas_source_context(&self) -> Option<Context> {
        if let Some(flat) = &self.document.flat_edit {
            let ready = flat.view.ready.as_ref()?;
            Some(Context {
                source: ready.developed.clone(),
                reference: ready.reference.clone(),
                correction: None,
            })
        } else {
            self.source_context()
        }
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
        let mut scan = Scan::with_reference(self.source.clone(), self.reference.clone());
        scan.set_flat_field(
            self.correction
                .as_ref()
                .map(|field| field.settings.map.clone()),
        )?;
        Ok(scan)
    }
}

pub(super) struct Request {
    pub input: crate::processing::InputSettings,
    pub reference: Arc<std::sync::OnceLock<crate::exposure_reference::ReferenceExposure>>,
    pub path: PathBuf,
    pub wb: Option<[f64; 4]>,
    pub expected: Option<Source>,
    pub sources: Option<Arc<dyn crate::source::SourceProvider>>,
    // New sources resolve identity alongside header inspection, off the UI thread.
    pub file: thumbnails::FileKey,
    pub correction: Option<Arc<Correction>>,
}

pub(super) struct Developed {
    pub context: Context,
    pub record: Source,
    file: thumbnails::FileKey,
    path: PathBuf,
    timing: Timing,
    camera_proxy: Option<Arc<crate::scan::CameraPixels>>,
}

#[derive(Clone)]
pub(super) struct Services {
    pub developments: source_loading::SourceLoader,
    pub renderers: Arc<crate::renderer::RendererCache>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Services {
    pub fn provider(
        &self,
        cache: Option<Arc<crate::source::SourceCache>>,
    ) -> Arc<dyn crate::source::SourceProvider> {
        self.developments.provider(cache)
    }

    pub async fn prepare(
        self,
        request: Request,
        cancelled: crate::cancellation::Cancellation,
    ) -> Result<Ready, String> {
        let sources = request
            .sources
            .clone()
            .unwrap_or_else(|| self.provider(None));
        let mut developed = self.clone().develop(request, cancelled.clone()).await?;
        if developed.camera_proxy.is_none() {
            let key = SourceKey::for_load(
                &developed.path,
                developed.file.clone(),
                &developed.record.development,
            );
            let proxy = sources.load_proxy(key, source_loading::Priority::Foreground);
            developed.camera_proxy = Some(
                futures_lite::future::or(
                    async {
                        cancelled.wait().await;
                        Err(Error::Cancelled)
                    },
                    proxy,
                )
                .await
                .map_err(|error| error.to_string())?,
            );
        }
        crate::background::run(move |_| {
            developed.prepare_gpu(&self.renderers, &self.device, &self.queue, &|| {
                cancelled.is_cancelled()
            })
        })
        .await
        .map_err(|_| "Source preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }

    /// Acquire captured inputs without preparing viewer resources.
    pub async fn acquire(
        self,
        request: Request,
        cancelled: crate::cancellation::Cancellation,
    ) -> Result<Context, String> {
        self.develop(request, cancelled)
            .await
            .map(|value| value.context)
    }

    async fn develop(
        self,
        mut request: Request,
        cancelled: crate::cancellation::Cancellation,
    ) -> Result<Developed, String> {
        let started = std::time::Instant::now();
        let retired = cancelled.clone();
        let sources = request
            .sources
            .take()
            .unwrap_or_else(|| self.provider(None));
        let needs_header = request.expected.is_none();
        let capture = move || {
            request.require_live(&|| retired.is_cancelled())?;
            let key = request.load_key()?;
            Ok::<_, Error>((request, key))
        };
        let captured = if needs_header {
            crate::background::run(move |_| capture())
                .await
                .map_err(|_| "Source header worker panicked".to_owned())?
        } else {
            capture()
        };
        let (request, key) = captured.map_err(|error| error.to_string())?;
        let file = key.file().clone();
        let load = crate::photo_preparation::acquire(
            sources.as_ref(),
            key,
            source_loading::Priority::Foreground,
            request.input,
            request.reference.clone(),
        );
        let prepared = futures_lite::future::or(
            async {
                cancelled.wait().await;
                Err(Error::Cancelled)
            },
            load,
        )
        .await
        .map_err(|error| error.to_string())?;
        let timing = Timing {
            started,
            development_seconds: started.elapsed().as_secs_f64(),
            registration_seconds: 0.,
            verification_seconds: 0.,
        };
        crate::background::run(move |_| {
            request.accept(prepared, file, &|| cancelled.is_cancelled(), timing)
        })
        .await
        .map_err(|_| "Source acceptance worker panicked".to_owned())?
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
        if cancelled() {
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
            self.file.clone()
        };
        Ok(SourceKey::for_load(&self.path, file, &record.development))
    }

    /// Synchronous fixtures drive the same provider, observing their poll-only stop signal.
    #[cfg(test)]
    pub fn load(
        mut self,
        developments: &source_loading::SourceLoader,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let key = self.load_key()?;
        let file = key.file().clone();
        let started = std::time::Instant::now();
        let provider = self
            .sources
            .take()
            .unwrap_or_else(|| developments.provider(None));
        let mut load = Box::pin(crate::photo_preparation::acquire(
            provider.as_ref(),
            key,
            source_loading::Priority::Foreground,
            self.input,
            self.reference.clone(),
        ));
        let prepared = loop {
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
        self.accept(prepared, file, cancelled, timing)
    }

    fn accept(
        self,
        prepared: crate::photo_preparation::PreparedSource,
        file: thumbnails::FileKey,

        cancelled: &dyn Fn() -> bool,
        mut timing: Timing,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let tick = std::time::Instant::now();
        let source = prepared.source.clone();
        let record = document::source_metadata(&self.path, &source.metadata(), source.fixed_wb())?;
        timing.registration_seconds = tick.elapsed().as_secs_f64();
        if let Some(expected) = &self.expected {
            document::verify_source(expected, &record)?;
        }
        let tick = std::time::Instant::now();
        let mut scan = Scan::from_source(source);
        document::verify_scan(&record, &scan)?;
        timing.verification_seconds = tick.elapsed().as_secs_f64();
        let camera_proxy = prepared.proxy;
        scan = Scan::with_reference(prepared.source, prepared.reference);
        self.require_live(cancelled)?;
        if let Some(correction) = &self.correction {
            correction.settings.require_source(&record.development)?;
        }
        let context = Context {
            source: scan.source().clone(),
            reference: scan.reference_exposure.clone(),
            correction: self.correction,
        };
        context.scan()?;
        Ok(Developed {
            context,
            record,
            file,
            path: self.path,
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
        if cancelled() {
            return Err(Error::UnusableSample("document source work retired"));
        }
        let scan = self.context.scan()?;
        let tick = std::time::Instant::now();
        let (width, height, pixels) = scan.preview_source(1024)?;
        let proxy = self
            .camera_proxy
            .map_or_else(|| thumbnails::Proxy::from_scan(&scan).map(Arc::new), Ok)?;
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
        )?
        .with_identity(crate::gpu::execution::source::Identity::camera(
            scan.source().identity(),
        ))?;
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
            reference: self.context.reference,
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
