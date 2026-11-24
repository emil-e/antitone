use super::*;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::{evaluation, lut3d::Looks, parameters::ProcessingParameters, submission::Queue};
#[cfg(test)]
use crate::project::SourceDevelopment;
pub(super) use crate::source::FileKey;
use crate::source::SourceKey as NativeKey;
use crate::thumbnails::{
    FieldKey, ImageKey, Outcome, Processed, Services, SourceKey, ThumbnailGpu, Worker, disk,
};
#[cfg(test)]
use crate::thumbnails::{
    Job, Product, REFRESH_QUIET_PERIOD, Stopped, Submission, ThumbnailMetrics, lru::Cache,
    shared_look_bytes,
};
mod embedded;
#[cfg(test)]
mod worker_tests;
#[cfg(test)]
mod workflow_tests;
pub(super) use crate::scan::CameraPixels as Proxy;

impl SourceKey {
    fn for_document(source: &Source, file: FileKey, document: &Document) -> Self {
        Self::new(source, file).with_development(
            document
                .source_input(source.id, &Default::default())
                .expect("registered photo input")
                .development()
                .expect("photo development"),
        )
    }
}

impl ImageKey {
    fn current_recipe(&self, document: &Document) -> bool {
        match &self.recipe {
            Ok(None) => true,
            Ok(Some(recipe)) => document
                .image_recipe(self.source.id)
                .is_ok_and(|current| &current == recipe.as_ref()),
            Err(reason) => document
                .image_recipe(self.source.id)
                .is_err_and(|error| error.to_string() == *reason),
        }
    }
}

pub(super) struct SelectedPreview {
    key: SourceKey,
    pub(super) proxy: crate::viewer::GpuProxy,
}

impl SelectedPreview {
    pub(super) fn area(&self) -> Rect {
        self.key.source.active_area.into()
    }

    pub(super) fn current_source(&self, document: &Document, source: Option<SourceId>) -> bool {
        source == Some(self.key.id)
            && document
                .project
                .source(self.key.id)
                .ok()
                .zip(document.source_path(self.key.id).ok())
                .is_some_and(|(source, path)| {
                    SourceKey::for_document(source, document.file_key(&path), document) == self.key
                })
    }
}

/// Filmstrip inputs captured by its caller for the current presentation.
#[derive(Default)]
pub(super) struct Preview {
    pub compositions: Vec<(SourceId, Composition)>,
    pub sources: Vec<(SourceId, Result<ProcessingParameters, String>)>,
}

#[cfg(test)]
impl ThumbnailGpu {
    fn fixture(
        device: &wgpu::Device,
        queue: &Queue,
        resources: &crate::editor::display::DisplayFixture,
    ) -> Self {
        assert_eq!(resources.evaluation().device(), device);
        Self {
            evaluation: resources.evaluation().clone(),
            queue: queue.clone(),
            looks: resources.looks().clone(),
            display: resources.warmup(queue),
        }
    }

    fn from_source_services(services: &super::source::Services) -> Self {
        Self {
            evaluation: services.evaluation.clone(),
            queue: services.queue.clone(),
            looks: services.looks.clone(),
            display: services.display.clone(),
        }
    }
}

pub(super) struct Thumbnails {
    pub(super) processed: Processed,
    pub(super) sources: Arc<crate::source::SourceCache>,
    selected_key: Option<NativeKey>,
    visible_keys: Vec<NativeKey>,
    budgets: memory::CacheBudgets,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    embedded: embedded::EmbeddedPreviews,
    backend: Option<Arc<dyn crate::source::SourceProvider>>,
    visible: Vec<SourceId>,
    preview: Preview,
}

impl Default for Thumbnails {
    fn default() -> Self {
        #[cfg(not(test))]
        let limits = cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        };
        #[cfg(test)]
        let limits = cache_limits::Hardware::current(None).defaults();
        let budgets = memory::CacheBudgets::new(limits, memory::Pressure::Normal);
        let repaint: Arc<std::sync::OnceLock<egui::Context>> = Arc::default();
        Self {
            processed: Processed::new(budgets.images, repaint.clone()),
            sources: Arc::new(crate::source::SourceCache::new(budgets.sources as u64)),
            selected_key: None,
            visible_keys: Vec::new(),
            budgets,
            repaint,
            embedded: Default::default(),
            backend: None,
            visible: Vec::new(),
            preview: Preview::default(),
        }
    }
}

impl Thumbnails {
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn preview(&self) -> &Preview {
        &self.preview
    }

    pub(super) fn selected_preview(
        &mut self,
        document: &Document,
        id: SourceId,
    ) -> Result<Option<SelectedPreview>, Error> {
        let started = std::time::Instant::now();
        let source = document.project.source(id)?;
        let key = SourceKey::for_document(
            source,
            document.file_key(&document.source_path(id)?),
            document,
        );
        if key.file.revision.is_none() {
            return Ok(None);
        }
        let Some(native_key) = key.store_key_at(&document.source_path(id)?) else {
            return Ok(None);
        };
        let Some(proxy) = self.sources.get_proxy(&native_key) else {
            return Ok(None);
        };
        if !document.project.input.is_negative() && document.reference_exposure(source).is_none() {
            return Ok(None);
        }
        let Some(gpu) = &self.processed.gpu else {
            return Ok(None);
        };
        let targets = gpu.display.ready()?;
        // Selected and filmstrip outputs have independent mutable processing state.
        let preview = SelectedPreview {
            key: key.clone(),
            proxy: crate::viewer::GpuProxy::new_continuous(
                gpu.evaluation.clone(),
                gpu.looks.clone(),
                targets,
                [proxy.width, proxy.height],
                &proxy.pixels,
                proxy.matrix,
                crate::gpu::source::SourceGeometry::new(
                    source.development.active_area,
                    [proxy.width, proxy.height],
                )?,
            )?
            .with_identity(crate::gpu::source::identity::Identity::proxy(&proxy))?,
        };
        if !preview.current_source(document, Some(id)) {
            return Ok(None);
        }
        self.selected_key = Some(native_key.clone());
        self.protect_cpu();
        let _ = self.sources.get_proxy(&native_key);
        let bytes = preview.proxy.byte_len();
        log::debug!(
            source_id = id.0,
            cache = "memory",
            width = proxy.width,
            height = proxy.height,
            gpu_pixel_bytes = bytes,
            preparation_ms = started.elapsed().as_secs_f64() * 1000.;
            "selected_preview"
        );
        Ok(Some(preview))
    }

    #[cfg(test)]
    pub fn seed(&mut self, source: &Source, file: FileKey, proxy: Arc<Proxy>) {
        let path = file.path.clone();
        self.seed_at_path(source, &path, file, proxy);
    }

    pub(super) fn seed_at_path(
        &mut self,
        source: &Source,
        path: &std::path::Path,
        file: FileKey,
        proxy: Arc<Proxy>,
    ) {
        let source_key = SourceKey::new(source, file);
        let Some(key) = source_key.store_key_at(path) else {
            return;
        };
        self.selected_key = Some(key.clone());
        self.protect_cpu();
        // Readmit a live candidate/selected native by-product after the accepted budget is bound.
        let _ = self.sources.get(&key);
        if let Err(error) = self.sources.publish_proxy(key, proxy) {
            log::warn!(error = error.to_string().as_str(); "selected_proxy_admission_rejected");
        }
    }

    pub(super) fn release_selected(&mut self) {
        self.selected_key = None;
        self.protect_cpu();
    }

    pub(super) fn refresh_hidden_memory(&mut self) {
        self.embedded.request_visible(&[]);
        self.processed.protect_images(Vec::new());
        self.visible_keys.clear();
        self.protect_cpu();
    }

    fn protect_cpu(&self) {
        let mut keys = self.visible_keys.clone();
        keys.extend(self.selected_key.clone());
        self.sources.protect_proxies(keys);
    }

    pub(super) fn for_sources(sources: Arc<crate::source::SourceCache>) -> Self {
        Self {
            sources,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn with_sources(mut self, sources: Arc<crate::source::SourceCache>) -> Self {
        sources.set_budget(self.budgets.sources as u64);
        self.sources = sources;
        self
    }

    pub(super) fn for_document(&self, sources: Arc<crate::source::SourceCache>) -> Self {
        let mut next = Self {
            sources,
            processed: self.processed.for_document(self.budgets.images),
            backend: self.backend.clone(),
            repaint: self.repaint.clone(),
            embedded: self.embedded.for_document(),
            ..Self::default()
        };
        next.budgets = self.budgets;
        next
    }

    pub(super) fn acquisition(&self) -> Option<Arc<dyn crate::source::SourceProvider>> {
        self.backend
            .as_ref()
            .map(|backend| self.sources.provider(backend.clone()))
    }

    #[cfg(test)]
    pub(super) fn default_limits(&self) -> cache_limits::CacheLimits {
        cache_limits::Hardware::current(
            self.processed
                .gpu
                .as_ref()
                .map(|gpu| gpu.evaluation.device()),
        )
        .defaults()
    }

    #[cfg(test)]
    pub(super) fn set_limits(&mut self, limits: cache_limits::CacheLimits) {
        self.apply_test_budgets(memory::CacheBudgets::new(limits, memory::Pressure::Normal));
    }

    #[cfg(test)]
    fn apply_test_budgets(&mut self, budgets: memory::CacheBudgets) {
        let sources = self.sources.clone();
        let evaluation = self
            .processed
            .gpu
            .as_ref()
            .map(|gpu| gpu.evaluation.clone());
        memory::apply_budgets(budgets, &sources, self, evaluation.as_ref());
    }

    pub(super) fn apply_budgets(&mut self, budgets: memory::CacheBudgets) -> usize {
        if budgets.pressure == memory::Pressure::Critical
            && self.budgets.pressure != memory::Pressure::Critical
        {
            self.processed.cancel();
            self.embedded.cancel();
        }
        self.budgets = budgets;
        self.embedded.set_budget(budgets.images.min(16 << 20));
        let embedded = self.embedded.byte_len();
        let processed = self
            .processed
            .set_budget(budgets.images.saturating_sub(embedded));
        embedded.saturating_add(processed)
    }

    pub fn new(
        evaluation: evaluation::Context,
        queue: Queue,
        looks: Arc<Looks>,
        display: Arc<super::display::Warmup>,
        developments: source_loading::SourceLoader,
    ) -> Self {
        Self::with_gpu(
            ThumbnailGpu {
                evaluation,
                queue,
                looks,
                display,
            },
            disk::DiskCache::default_location(),
            developments,
        )
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn without_disk(&self, developments: source_loading::SourceLoader) -> Self {
        let gpu = self.processed.gpu.clone().expect("thumbnail GPU resources");
        Self::with_gpu(gpu, None, developments)
    }

    #[cfg(test)]
    pub(super) fn without_disk_for_source(services: &super::source::Services) -> Self {
        Self::with_gpu(
            ThumbnailGpu::from_source_services(services),
            None,
            services.developments.clone(),
        )
    }

    #[cfg(test)]
    fn with_disk(
        device: wgpu::Device,
        queue: crate::gpu::submission::Queue,
        renderers: Arc<crate::editor::display::DisplayFixture>,
        disk: Option<disk::DiskCache>,
    ) -> Self {
        Self::with_developments(
            device,
            queue,
            renderers,
            disk,
            source_loading::SourceLoader::new(),
        )
    }

    #[cfg(test)]
    fn with_developments(
        device: wgpu::Device,
        queue: Queue,
        renderers: Arc<crate::editor::display::DisplayFixture>,
        disk: Option<disk::DiskCache>,
        developments: source_loading::SourceLoader,
    ) -> Self {
        Self::with_gpu(
            ThumbnailGpu::fixture(&device, &queue, &renderers),
            disk,
            developments,
        )
    }

    fn with_gpu(
        gpu: ThumbnailGpu,
        disk: Option<disk::DiskCache>,
        developments: source_loading::SourceLoader,
    ) -> Self {
        let mut state = Self::default();
        state.processed.gpu = Some(gpu.clone());
        #[cfg(test)]
        state.set_limits(state.default_limits());
        state.embedded = embedded::EmbeddedPreviews::new();
        state.backend = Some(Arc::new(developments));
        state.processed.services = Some(Services::Production(Worker::new(
            gpu,
            disk,
            state.repaint.clone(),
        )));
        state
    }

    #[cfg(test)]
    pub(super) fn ready_count(&self, document: &Document, target: OutputTarget) -> usize {
        self.processed
            .cache
            .entries
            .iter()
            .filter(|(key, value)| {
                value.is_ok()
                    && key.target == target
                    && key.current_recipe(document)
                    && key
                        .recipe
                        .as_ref()
                        .is_ok_and(|recipe| recipe.is_some() == document.project.input.ready())
                    && key.field.as_ref().map(|f| &f.0)
                        == document.project.active_flat_field().map(|f| &f.map)
                    && document
                        .project
                        .source(key.source.id)
                        .ok()
                        .is_some_and(|source| {
                            SourceKey::for_document(
                                source,
                                document.file_key(
                                    &document.source_path(source.id).expect("thumbnail locator"),
                                ),
                                document,
                            ) == key.source
                        })
            })
            .count()
    }

    fn complete(&mut self, outcome: Outcome, document: &Document) -> bool {
        let source_current = document
            .project
            .source(outcome.key.source.id)
            .ok()
            .is_some_and(|source| {
                SourceKey::for_document(
                    source,
                    document.file_key(&document.source_path(source.id).expect("thumbnail locator")),
                    document,
                ) == outcome.key.source
            });
        self.processed.complete(outcome, source_current, |key| {
            key.current_recipe(document)
                && key.field.as_ref().map(|f| &f.0)
                    == document.project.active_flat_field().map(|f| &f.map)
        })
    }
}

impl Editor {
    fn thumbnail_key(&self, source: &Source, path: &std::path::Path) -> Option<ImageKey> {
        let recipe = self.thumbnail_recipe(source.id);
        Some(ImageKey {
            source: SourceKey::new(source, self.document.file_key(path))
                .with_development(self.document.development_for(source, self.input())),
            recipe,
            target: self.document.target(),
            field: self.correction().map().cloned().map(FieldKey),
        })
    }

    fn thumbnail_composition(&self, source: &Source) -> Composition {
        let photo_orientation = if self.document.visit.current_source == Some(source.id) {
            self.photo_orientation()
        } else {
            source.orientation
        };
        if let Some((_, composition)) = self
            .document
            .thumbnails
            .preview
            .compositions
            .iter()
            .find(|(id, _)| *id == source.id)
        {
            return *composition;
        }
        let crop = if self.document.visit.current_source == Some(source.id) {
            self.photo_crop()
        } else {
            source.crop
        };
        if crop.is_some() {
            Composition::with_crop(
                source.development.active_area.into(),
                self.document.project.orientation,
                photo_orientation,
                crop,
            )
        } else {
            Ok(self.source_composition(source))
        }
        .expect("document source has a valid active area")
    }

    fn thumbnail_recipe(
        &self,
        id: SourceId,
    ) -> Result<Option<Arc<crate::image_recipe::ImageRecipe>>, String> {
        if self
            .document
            .thumbnails
            .preview
            .sources
            .iter()
            .any(|(source, _)| *source == id)
            || !self.input().ready()
        {
            Ok(None)
        } else {
            let source = self
                .document
                .project
                .source(id)
                .map_err(|error| error.to_string())?;
            self.thumbnail_request(source)
                .map(|request| request.recipe().cloned())
        }
    }

    fn thumbnail_request(
        &self,
        source: &Source,
    ) -> Result<crate::gpu::photograph::request::PhotographRequest, String> {
        use crate::gpu::photograph::request::PhotographRequest;
        if let Some((_, parameters)) = self
            .document
            .thumbnails
            .preview
            .sources
            .iter()
            .find(|(id, _)| *id == source.id)
        {
            return parameters
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|parameters| {
                    PhotographRequest::inspection(*parameters).map_err(|error| error.to_string())
                });
        }
        if !self.input().ready() {
            return self
                .source_parameters_for(source, self.source_exposure)
                .and_then(PhotographRequest::inspection)
                .map_err(|error| error.to_string());
        }
        self.positive_recipe_for_source(
            self.input(),
            self.document.project_snapshot().roll,
            source,
            self.photo_settings(),
        )
        .map(|recipe| PhotographRequest::Recipe(Arc::new(recipe)))
        .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    fn thumbnail_parameters(&self, source: &Source) -> Result<AuthoredPhotograph, String> {
        self.thumbnail_request(source)?
            .processing(
                &self
                    .document
                    .visit
                    .working_view
                    .device()
                    .map(|device| device.limits())
                    .unwrap_or_default(),
            )
            .map_err(|error| error.to_string())
    }
}

pub(super) fn prepare_filmstrip_images(
    editor: &mut Editor,
    preview: Preview,
    context: &egui::Context,
) {
    let thumbnails = &mut editor.document.thumbnails;
    thumbnails.preview = preview;
    thumbnails.repaint.get_or_init(|| context.clone());
    editor.apply_cache_budgets();
    if editor.document.visit.canvas.ready.is_none()
        && editor.document.visit.pending_preview.is_none()
    {
        editor.document.thumbnails.release_selected();
    }

    let field = editor.correction().map().cloned().map(FieldKey);
    editor.document.thumbnails.processed.retain_field(field);
}

#[derive(Default)]
pub(super) struct ThumbnailProvider {
    visible: Vec<SourceId>,
    protected: Vec<ImageKey>,
    camera_wanted: Vec<(usize, ImageKey, PathBuf)>,
    awaiting_correction: bool,
}

pub(super) struct NavigationImage<'a> {
    pub photograph: Option<Result<NavigationPhotograph<'a>, String>>,
    pub camera: Option<&'a egui::TextureHandle>,
    pub updating: bool,
}

pub(super) struct NavigationPhotograph<'a> {
    image: Arc<GpuImage>,
    parameters: Option<crate::gpu::photograph::request::PhotographRequest>,
    pub composition: Composition,
    correction: correction::Resources,
    presentation: &'a Result<Presentation, Error>,
}

impl NavigationPhotograph<'_> {
    pub(super) fn paint(
        self,
        mapping: crate::geometry::TextureMapping,
    ) -> Result<Option<ImagePaint>, String> {
        if self.parameters.is_some() && self.correction.pending() {
            return Ok(None);
        }
        let field = match self.parameters.as_ref() {
            Some(request) if request.is_inspection() && self.correction.error().is_some() => {
                Ok(None)
            }
            Some(_) => self.correction.gpu(),
            None => Ok(None),
        }
        .map_err(|error| error.to_string())?;
        let presentation = self.presentation.as_ref().map_err(ToString::to_string)?;
        processing::image_paint(presentation, self.image, self.parameters, field, mapping)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

impl ThumbnailProvider {
    pub(super) fn lookup<'a>(
        &mut self,
        editor: &'a Editor,
        source: &Source,
        path: &std::path::Path,
    ) -> NavigationImage<'a> {
        self.visible.push(source.id);
        let Some(key) = editor.thumbnail_key(source, path) else {
            return NavigationImage {
                photograph: None,
                camera: None,
                updating: false,
            };
        };
        let thumbnails = &editor.document.thumbnails;
        let camera = thumbnails.embedded.get(&key.source);
        let thumbnail = thumbnails.processed.image(&key);
        let updating = camera.is_none()
            && match &thumbnail {
                Some(Ok(thumbnail)) => !thumbnail.current,
                Some(Err(_)) => false,
                None => true,
            };
        if thumbnail.is_none() {
            self.camera_wanted
                .push((self.visible.len() - 1, key.clone(), path.to_owned()));
        }
        self.protected.push(key);

        let photograph = thumbnail.map(|result| {
            let thumbnail = result?;
            let correction = editor.correction();
            let parameters = if thumbnail.graded {
                None
            } else {
                let request = editor.thumbnail_request(source)?;
                self.awaiting_correction |= correction.pending();
                Some(request)
            };
            Ok(NavigationPhotograph {
                image: thumbnail.image,
                parameters,
                composition: editor.thumbnail_composition(source),
                correction,
                presentation: &editor.presentation,
            })
        });
        NavigationImage {
            photograph,
            camera,
            updating,
        }
    }

    pub(super) fn publish_demand(
        self,
        editor: &mut Editor,
        context: &egui::Context,
        cx: &UiContext<'_, Editor>,
    ) {
        let Self {
            visible,
            protected,
            camera_wanted,
            awaiting_correction,
        } = self;
        editor
            .document
            .thumbnails
            .processed
            .touch_visible(&protected);
        if awaiting_correction {
            editor.document.correction.observe(cx);
        }

        let thumbnails = &mut editor.document.thumbnails;
        thumbnails.visible = visible;
        thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        thumbnails.protect_cpu();
        thumbnails.processed.protect_images(protected.clone());
        editor.apply_cache_budgets();
        editor
            .document
            .thumbnails
            .embedded
            .request_visible(&camera_wanted);
        editor.apply_cache_budgets();
        start_next_embedded_preview(editor, context, cx);
        editor.schedule_with_visible_keys(cx, std::time::Instant::now(), false, protected);
        if editor.document.thumbnails.processed.has_pending()
            || editor.document.thumbnails.embedded.pending()
        {
            context.request_repaint_after(Duration::from_millis(50));
        }
    }
}

pub(super) fn clear_visible_photos(
    editor: &mut Editor,
    context: &egui::Context,
    cx: &UiContext<'_, Editor>,
) {
    let thumbnails = &mut editor.document.thumbnails;
    thumbnails.repaint.get_or_init(|| context.clone());
    thumbnails.visible.clear();
    thumbnails.refresh_hidden_memory();
    editor.apply_cache_budgets();
    editor.advance_thumbnail_demand(cx);
    if editor.document.thumbnails.processed.has_pending() {
        context.request_repaint_after(Duration::from_millis(50));
    }
}

fn start_next_embedded_preview(
    editor: &mut Editor,
    context: &egui::Context,
    cx: &UiContext<'_, Editor>,
) {
    let document = &mut editor.document;
    if document.thumbnails.budgets.pressure == memory::Pressure::Critical {
        document.thumbnails.embedded.cancel();
        return;
    }

    let sources = document.thumbnails.acquisition();
    let presentation = context.clone();
    document.thumbnails.embedded.start(
        &document.state,
        sources,
        &document.initialization,
        context,
        cx,
        move |cx, finished| {
            cx.update(|editor| {
                let document = &mut editor.document;
                if document
                    .thumbnails
                    .embedded
                    .complete(finished, &document.state, &presentation)
                {
                    editor.apply_cache_budgets();
                    start_next_embedded_preview(editor, &presentation, &cx);
                }
            });
        },
    );
}

impl Editor {
    fn request_thumbnail(
        &mut self,
        key: ImageKey,
        render: bool,
        priority: source_loading::Priority,
        cx: &UiContext<'_, Self>,
    ) {
        let physical = self
            .document
            .source_input(key.source.id)
            .expect("registered thumbnail input");
        let correction = self.correction();
        let sources = self.document.thumbnails.acquisition();
        let processed = &mut self.document.thumbnails.processed;
        let Some((request, task)) =
            processed.request(key, physical, correction, render, priority, sources)
        else {
            return;
        };
        let completion = task.id.clone();
        let ticket = task.id.ticket;
        let fallback = task.key.clone();
        let Some(continuation) = cx.spawn(move |cx| async move {
            let result = request.await;
            cx.update(|editor| {
                let Some(pending) = editor.document.thumbnails.processed.take_work(&completion) else {
                    return;
                };
                editor.document.state.capture_initialization(&editor.document.initialization);
                let rendered = match result {
                    Ok(mut outcome) => {
                        if let Some(proxy) = outcome.proxy.take()
                            && let Some(key) = outcome.key.source.store_key()
                            && let Err(error) = editor.document.cache.publish_proxy(key, proxy)
                        {
                            log::warn!(error = error.to_string().as_str(); "thumbnail_proxy_publication_failed");
                        }
                        let current = editor.document.effective_document();
                        editor.document.thumbnails.complete(outcome, &current)
                    }
                    Err(_) => {
                        editor.document.thumbnails.processed.stopped(ticket, fallback);
                        false
                    }
                };
                drop(pending);
                editor.apply_cache_budgets();
                editor.schedule_thumbnail(&cx, std::time::Instant::now(), rendered);
            });
        }) else {
            processed.stopped(task.id.ticket, task.key);
            return;
        };
        processed.track(task, continuation);
    }

    pub(super) fn advance_thumbnail_demand(&mut self, cx: &UiContext<'_, Self>) {
        self.schedule_thumbnail(cx, std::time::Instant::now(), false);
        if let Some(context) = self.document.thumbnails.repaint.get().cloned() {
            start_next_embedded_preview(self, &context, cx);
        }
    }

    fn schedule_thumbnail(
        &mut self,
        cx: &UiContext<'_, Self>,
        now: std::time::Instant,
        continuing: bool,
    ) {
        let mut protected = Vec::new();
        for id in &self.document.thumbnails.visible {
            if let Ok(source) = self.document.state.project.source(*id)
                && let Ok(path) = self.document.source_path(*id)
                && let Some(key) = self.thumbnail_key(source, &path)
            {
                protected.push(key);
            }
        }
        self.schedule_with_visible_keys(cx, now, continuing, protected);
    }

    fn schedule_with_visible_keys(
        &mut self,
        cx: &UiContext<'_, Self>,
        now: std::time::Instant,
        continuing: bool,
        protected: Vec<ImageKey>,
    ) {
        let visible = self.document.thumbnails.visible.clone();
        let pressure = self.document.thumbnails.budgets.pressure;
        let eager = pressure == memory::Pressure::Normal;
        if let Some(key) = self.document.thumbnails.processed.pending_key() {
            let visible = visible.contains(&key.source.id);
            let current = if visible {
                protected.contains(key)
            } else {
                self.document
                    .project
                    .source(key.source.id)
                    .ok()
                    .zip(self.document.source_path(key.source.id).ok())
                    .and_then(|(source, path)| self.thumbnail_key(source, &path))
                    .as_ref()
                    == Some(key)
            };
            self.document
                .thumbnails
                .processed
                .reconcile(current, visible, eager, &protected);
        }
        self.document.thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        self.document.thumbnails.protect_cpu();
        self.document
            .thumbnails
            .processed
            .protect_images(protected.clone());
        self.apply_cache_budgets();
        let processed = &mut self.document.thumbnails.processed;
        if processed.mark_unavailable(&protected)
            || pressure == memory::Pressure::Critical
            || processed.has_pending()
        {
            return;
        }
        let visible_indices: Vec<_> = self
            .document
            .project
            .order
            .iter()
            .enumerate()
            .filter_map(|(index, id)| visible.contains(id).then_some(index))
            .collect();
        let mut candidates: Vec<_> = (0..self.document.state.project.order.len())
            .filter(|index| visible_indices.contains(index) || eager)
            .collect();
        candidates.sort_by_key(|index| !visible_indices.contains(index));
        for index in candidates {
            let id = self.document.state.project.order[index];
            let visible = visible.contains(&id);
            let key = if visible {
                protected.iter().find(|key| key.source.id == id).cloned()
            } else {
                let source = self
                    .document
                    .project
                    .source(id)
                    .expect("thumbnail demand source");
                let path = self
                    .document
                    .source_path(id)
                    .expect("thumbnail demand locator");
                self.thumbnail_key(source, &path)
            };
            let Some(key) = key else {
                continue;
            };
            let render = match self
                .document
                .thumbnails
                .processed
                .next(&key, visible, continuing, now)
            {
                crate::thumbnails::Demand::Skip => continue,
                crate::thumbnails::Demand::Wait => return,
                crate::thumbnails::Demand::Request { render } => render,
            };
            self.request_thumbnail(
                key,
                render,
                if visible {
                    source_loading::Priority::Visible
                } else {
                    source_loading::Priority::Eager
                },
                cx,
            );
            return;
        }
        self.document.thumbnails.processed.clear_refresh();
    }
}

#[cfg(test)]
mod clipping_tests;
#[cfg(all(test, target_os = "macos"))]
use crate::cache_observation as observation;
#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod recipe_tests;
#[cfg(test)]
mod renderer_tests;
#[cfg(test)]
mod revision_tests;
#[cfg(test)]
mod selected_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod source_cache_tests;
#[cfg(test)]
mod style_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "../../native/editor_switching_observation.rs"]
mod switch_observation;
#[cfg(test)]
mod tests;
