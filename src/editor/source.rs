//! Explicit source acquisition and preparation. Each consumer leases immutable
//! development and carries its own correction; no selected-source slot is needed.
use super::{
    ActivityOptions, Editor, Ready, SelectionTiming, UiContext, document, source_loading,
    thumbnails,
};
use crate::{
    Error,
    project::Source,
    scan::{DevelopedSource, Scan},
};
use std::{path::PathBuf, sync::Arc};

#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

#[derive(Clone)]
pub(super) struct Context {
    pub source: Arc<DevelopedSource>,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
    pub correction: super::correction::Resources,
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
        let correction = self.correction();
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
            context.correction = correction.acquire().await?.into();
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
        let correction = self.correction();
        Some(Context {
            source: ready.developed.clone(),
            reference: ready.reference.clone(),
            correction: if correction.error().is_some() {
                Default::default()
            } else {
                correction
            },
        })
    }
}

impl PartialEq for Context {
    fn eq(&self, other: &Self) -> bool {
        self.source.identity() == other.source.identity()
            && self.reference == other.reference
            && self.correction.map() == other.correction.map()
    }
}

impl Context {
    pub fn scan(&self) -> Result<Scan, Error> {
        let mut scan = Scan::with_reference(self.source.clone(), self.reference.clone());
        scan.set_flat_field(self.correction.map().cloned());
        Ok(scan)
    }
}

pub(super) struct Request {
    pub source: crate::source::Input,
    pub sources: Option<Arc<dyn crate::source::SourceProvider>>,
    pub correction: super::correction::Resources,
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
    pub evaluation: crate::gpu::evaluation::Context,
    pub looks: Arc<crate::gpu::lut3d::Looks>,
    pub display: Arc<super::display::Warmup>,
    pub device: wgpu::Device,
    pub queue: crate::gpu::submission::Queue,
}

impl Services {
    #[cfg(test)]
    pub fn fixture(
        developments: source_loading::SourceLoader,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        resources: &crate::editor::display::DisplayFixture,
    ) -> Self {
        Self {
            developments,
            evaluation: resources.evaluation().clone(),
            looks: resources.looks().clone(),
            display: resources.warmup(queue),
            device: device.clone(),
            queue: queue.clone(),
        }
    }

    #[cfg(test)]
    pub fn render_fixture(&self) -> Result<Arc<crate::render_fixture::RenderFixture>, Error> {
        Ok(Arc::new(crate::render_fixture::RenderFixture {
            evaluation: self.evaluation.clone(),
            looks: self.looks.clone(),
            drt: self.display.ready()?,
        }))
    }

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
            developed.prepare_gpu(self.evaluation, self.looks, &self.display, &|| {
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
        evaluation: crate::gpu::evaluation::Context,
        looks: Arc<crate::gpu::lut3d::Looks>,
        display: &super::display::Warmup,
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
        let targets = display.ready()?;
        let renderer_seconds = tick.elapsed().as_secs_f64();
        let tick = std::time::Instant::now();
        let a = scan.area;
        let proxy = crate::viewer::GpuProxy::new_continuous(
            evaluation,
            looks,
            targets,
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

    pub input: Option<crate::source::Input>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.task.is_some()
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
            correction: self.correction(),
            sources: self.source_provider(),
        }
    }

    pub(super) fn activate_photo(
        &mut self,
        selected: Option<crate::project::SourceId>,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.visit.current_source == selected {
            return;
        }
        let Some(selected) = selected else {
            self.set_view_mode(super::viewer_ui::Mode::Photo);
            self.document.visit.working_view.clear();
            self.replace_photo_scope(None);
            self.document
                .roll_selection
                .reconcile(&self.document.state.project.order, None);
            self.document.thumbnails.release_selected();
            self.document.thumbnails.clear_visible();
            self.document.visit.canvas.viewport.fit();
            self.status = "No photos in this roll. Add scans to continue.".into();
            return;
        };
        let path = match self.document.source_path(selected) {
            Ok(path) => path,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.load_source(Some(selected), path, cx);
    }

    pub(super) fn browse_photo(
        &mut self,
        selected: crate::project::SourceId,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.visit.current_source == Some(selected) {
            return;
        }
        self.activate_photo(Some(selected), cx);
        self.document.visit.canvas.viewport.fit();
        self.document.source =
            !self.document.project.input.ready() || self.correction().error().is_some();
    }

    #[cfg(test)]
    pub(super) fn load(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        self.load_source(self.document.find_source(&path), path, cx);
        self.document.visit.canvas.viewport.fit();
        self.document.source =
            !self.document.project.input.ready() || self.correction().error().is_some();
    }

    fn load_source(
        &mut self,
        selected: Option<crate::project::SourceId>,
        path: PathBuf,
        cx: &UiContext<'_, Self>,
    ) {
        let started = std::time::Instant::now();
        self.seed_thumbnail();
        self.replace_photo_scope(selected);
        self.document.roll_selection.reconcile(
            &self.document.state.project.order,
            self.document.visit.current_source,
        );
        self.document.visit.canvas.last_layout = None;
        self.document.export.form = None;
        #[cfg(test)]
        {
            self.document.export.summary = None;
        }
        self.document.visit.return_notice = None;
        self.document.visit.canvas.ready = None;
        self.document.visit.canvas.selection_timing =
            Some(Arc::new(SelectionTiming::new(selected, started)));
        self.status = "Developing full-resolution RAW…".into();

        self.start_source_acquisition(path, cx);
        if let Some(source) = self.document.visit.current_source {
            match self
                .document
                .thumbnails
                .selected_preview(&self.document.state, source)
            {
                Ok(preview) => {
                    log::debug!(source_id = source.0, cache_hit = preview.is_some(), boundary = "admission", elapsed_ms = started.elapsed().as_secs_f64() * 1000.; "selected_switch");
                    self.document.visit.pending_preview = preview;
                }
                Err(error) => {
                    log::debug!(source_id = source.0, error = error.to_string().as_str(); "selected_preview_unavailable")
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
            cx.update(|editor| editor.source_result(result));
        }) else {
            self.source_result(Err("Application stopped".into()));
            return;
        };
        self.document.visit.source.task = Some(task);
    }

    fn source_result(&mut self, result: Result<Box<Ready>, String>) {
        let source = self.document.visit.current_source.map(|source| source.0);
        self.document.visit.source = State::default();

        if result.is_err() {
            self.document.visit.pending_preview = None;
        }
        if let Some(timing) = &self.document.visit.canvas.selection_timing {
            log::debug!(
                source_id = source,
                success = result.is_ok(),
                boundary = "source_ready",
                elapsed_ms = timing.started.elapsed().as_secs_f64() * 1000.;
                "selected_switch"
            );
        }

        match result {
            Ok(ready) => {
                log::info!(
                    source_id = source,
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
                    source_id = source,
                    error = error.as_str();
                    "source_load_failed"
                );

                self.document.visit.canvas.selection_timing = None;
                self.status = error;
            }
        }
    }
}
