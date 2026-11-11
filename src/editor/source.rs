//! Explicit source acquisition and preparation. Each consumer leases immutable
//! development and carries its own correction; no selected-source slot is needed.
use super::{
    ActivityOptions, AdjustmentLayerEditor, Correction, Editor, Ready, SelectionTiming, UiContext,
    document, source_loading, thumbnails,
};
use crate::{
    Error,
    project::Source,
    scan::{DevelopedSource, Scan},
};
use std::{path::PathBuf, sync::Arc};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

#[derive(Clone)]
pub(super) struct Context {
    pub source: Arc<DevelopedSource>,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
    pub correction: Option<Arc<Correction>>,
}

impl Editor {
    pub(super) fn selected_area(&self) -> Option<crate::raw::Rect> {
        self.document
            .visit
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .map(|source| source.development.active_area.into())
            .or_else(|| {
                self.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .map(|ready| ready.area)
            })
    }

    pub(super) fn capture_source_context(
        &self,
    ) -> Result<impl Future<Output = Result<Context, String>> + use<>, String> {
        let correction = self.document.correction.clone();
        let ready = self.source_context();
        let request = if ready.is_none() {
            let id = self
                .document
                .visit
                .current_source
                .ok_or("No selected photo")?;
            let path = self
                .document
                .source_path(id)
                .map_err(|error| error.to_string())?;
            let services = self
                .source_services
                .clone()
                .ok_or("Source services unavailable")?;
            Some((services, self.source_input(path)))
        } else {
            None
        };

        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        // Keep acquisition state off callers' stacks, including the already-ready fast path.
        Ok(Box::pin(async move {
            let _retire = retire;
            let mut context = if let Some(ready) = ready {
                ready
            } else {
                let (services, request) = request.expect("captured selected source");
                services.acquire(request, cancelled.clone()).await?
            };
            context.correction = correction.acquire().await?;
            Ok(context)
        }))
    }

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
            correction: self.document.correction.prepared().cloned(),
        })
    }
}

impl PartialEq for Context {
    fn eq(&self, other: &Self) -> bool {
        self.source.identity() == other.source.identity()
            && self.reference == other.reference
            && self
                .correction
                .as_ref()
                .map(|correction| &correction.settings.map)
                == other
                    .correction
                    .as_ref()
                    .map(|correction| &correction.settings.map)
    }
}

impl Context {
    pub fn scan(&self) -> Result<Scan, Error> {
        let mut scan = Scan::with_reference(self.source.clone(), self.reference.clone());
        scan.set_flat_field(
            self.correction
                .as_ref()
                .map(|field| field.settings.map.clone()),
        );
        Ok(scan)
    }
}

pub(super) struct Request {
    pub source: crate::source::Input,
    pub sources: Option<Arc<dyn crate::source::SourceProvider>>,
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
        let physical = request.source.clone();
        let sources = request
            .sources
            .clone()
            .unwrap_or_else(|| self.provider(None));
        let mut developed = self.clone().develop(request, cancelled.clone()).await?;
        if developed.camera_proxy.is_none() {
            let proxy = physical.proxy(sources.as_ref(), source_loading::Priority::Foreground);
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
        let sources = request
            .sources
            .take()
            .unwrap_or_else(|| self.provider(None));
        let load = request
            .source
            .source(sources.as_ref(), source_loading::Priority::Foreground);
        let acquired = futures_lite::future::or(
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
            request.accept(acquired, &|| cancelled.is_cancelled(), timing)
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

    /// Synchronous fixtures drive the same provider, observing their poll-only stop signal.
    #[cfg(test)]
    pub fn load(
        mut self,
        developments: &source_loading::SourceLoader,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let started = std::time::Instant::now();
        let provider = self
            .sources
            .take()
            .unwrap_or_else(|| developments.provider(None));
        let mut load = Box::pin(
            self.source
                .source(provider.as_ref(), source_loading::Priority::Foreground),
        );
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
        self.accept(prepared, cancelled, timing)
    }

    fn accept(
        self,
        acquired: crate::source::input::Acquired,
        cancelled: &dyn Fn() -> bool,
        mut timing: Timing,
    ) -> Result<Developed, Error> {
        self.require_live(cancelled)?;
        let tick = std::time::Instant::now();
        let crate::source::input::Acquired {
            prepared,
            record,
            file,
            ..
        } = acquired;
        let source = prepared.source.clone();
        let actual = document::source_metadata(
            self.source.path(),
            &source.metadata(),
            source.white_balance(),
        )?;
        timing.registration_seconds = tick.elapsed().as_secs_f64();
        document::verify_source(&record, &actual)?;
        let tick = std::time::Instant::now();
        let mut scan = Scan::from_source(source);
        document::verify_scan(&record, &scan)?;
        timing.verification_seconds = tick.elapsed().as_secs_f64();
        let camera_proxy = prepared.proxy;
        scan = Scan::with_reference(prepared.source, prepared.reference);
        self.require_live(cancelled)?;
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
            path: self.source.path().to_owned(),
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
            crate::gpu::source::SourceGeometry::new(
                [a.x, a.y, a.width, a.height],
                [width, height],
            )?,
        )?
        .with_identity(crate::gpu::source::identity::Identity::camera(
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

#[derive(Default)]
pub(super) struct State {
    task: Option<async_executor::Task<()>>,
    completion: Option<Result<Box<Ready>, String>>,

    pub input: Option<crate::source::Input>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.task.is_some() || self.completion.is_some()
    }
}

impl Editor {
    pub(super) fn source_input(&self, path: PathBuf) -> Request {
        let source = self
            .document
            .find_source(&path)
            .map(|id| {
                self.document
                    .source_input(id)
                    .expect("registered source input")
            })
            .unwrap_or_else(|| {
                crate::source::Input::reference(
                    path.clone(),
                    self.document.file_key(&path),
                    self.document.project.acquisition_wb(),
                    self.input(),
                    Arc::default(),
                )
            });
        Request {
            source,
            correction: self.document.correction.prepared().cloned(),
            sources: self.source_provider(),
        }
    }

    pub(super) fn load(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        let started = std::time::Instant::now();
        let selecting_photo = !self.picking();
        self.seed_thumbnail();
        self.document.visit.pending_preview = None;
        self.document.visit.canvas.selection_timing = None;
        self.document.thumbnails.cancel();
        self.document.visit.adjustment_layers = AdjustmentLayerEditor::default();
        self.id = self.request_id();
        self.replace_photo_scope(self.id);
        self.document.visit.current_source = self.document.find_source(&path);
        if selecting_photo {
            self.document.roll_selection.reconcile(
                &self.document.state.project.order,
                self.document.visit.current_source,
            );
        }
        self.refresh_adjustment_layer_selection();
        self.document.visit.canvas.last_layout = None;
        self.invalidate_original();
        self.document.visit.canvas.viewport.fit();
        self.document.export.draft = None;
        #[cfg(test)]
        {
            self.document.export.summary = None;
        }
        self.document.visit.return_notice = None;
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.document.visit.canvas.ready = None;
        self.document.visit.canvas.selection_timing =
            selecting_photo.then(|| Arc::new(SelectionTiming::new(self.id, started)));
        self.document.source =
            !self.document.project.input.ready() || self.document.correction.error().is_some();
        self.status = "Developing full-resolution RAW…".into();

        self.start_source_acquisition(path, cx);
        if selecting_photo
            && !self.picking()
            && let Some(source) = self.document.visit.current_source
        {
            match self
                .document
                .thumbnails
                .selected_preview(self.id, &self.document.state, source)
            {
                Ok(preview) => {
                    log::debug!(request = self.id, cache_hit = preview.is_some(), boundary = "admission", elapsed_ms = started.elapsed().as_secs_f64() * 1000.; "selected_switch");
                    self.document.visit.pending_preview = preview;
                }
                Err(error) => {
                    log::debug!(request = self.id, error = error.to_string().as_str(); "selected_preview_unavailable")
                }
            }
        }
    }

    pub(super) fn start_source_acquisition(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        let activity = self
            .activities
            .start(ActivityOptions::new("Loading photo…"));
        let request = self.source_input(path);
        self.document.visit.source.input = Some(request.source.clone());
        let view = self.document.visit.view;
        let services = self.source_services.clone();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => services.prepare(request, cancelled).await.map(Box::new),
                None => Err("Source services unavailable".into()),
            };
            drop(activity);
            cx.update(|editor| editor.source_result(view, result));
        }) else {
            self.source_result(view, Err("Application stopped".into()));
            return;
        };
        self.document.visit.source.task = Some(task);
    }

    pub(super) fn source_result(&mut self, view: u64, result: Result<Box<Ready>, String>) {
        if self.document.visit.view != view {
            return;
        }
        self.document.visit.source.task = None;
        self.document.visit.source.completion = Some(result);
        self.resume_source_completion();
    }

    fn resume_source_completion(&mut self) {
        if self.document_pending.is_some() {
            return;
        }
        let Some(result) = self.document.visit.source.completion.take() else {
            return;
        };
        let view = self.document.visit.view;
        self.document.visit.source = State::default();

        if result.is_err() {
            self.document.visit.pending_preview = None;
        }
        if let Some(timing) = &self.document.visit.canvas.selection_timing {
            log::debug!(
                request = view,
                success = result.is_ok(),
                boundary = "source_ready",
                elapsed_ms = timing.started.elapsed().as_secs_f64() * 1000.;
                "selected_switch"
            );
        }

        match result {
            Ok(ready) => {
                log::info!(
                    request = view,
                    width = ready.area.width,
                    height = ready.area.height;
                    "source_ready"
                );
                if self.document.visit.current_source.is_none() {
                    let mut record = ready.record.clone();
                    self.overlay_adjustment_layer_settings(&mut record, self.photo_settings());
                    record.orientation = self.photo_orientation();
                    record.crop = self.photo_crop();
                    match self.document.register(record) {
                        Ok(id) => self.document.visit.current_source = Some(id),
                        Err(error) => {
                            self.status = error.to_string();
                            return;
                        }
                    }
                }
                self.status = format!(
                    "{} • {} × {}",
                    ready.path.file_name().unwrap_or_default().to_string_lossy(),
                    ready.area.width,
                    ready.area.height
                );
                if let Some(notice) = self.document.visit.return_notice.take() {
                    self.status = notice;
                }
                self.document.cache_ready(&ready);
                self.document.visit.canvas.ready = Some(*ready);
            }
            Err(error) => {
                log::warn!(
                    request = view,
                    error = error.as_str();
                    "source_load_failed"
                );

                self.document.visit.canvas.selection_timing = None;
                self.status = error;
            }
        }
    }

    // Candidate failure and Save retain the requested visit. A result that arrived
    // during preparation becomes usable without another load or another paint.
    pub(super) fn resume_source_after_document(&mut self) {
        self.resume_source_completion();
    }
}
