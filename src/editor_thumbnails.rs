use super::presentation::ContactSheetLayout;
use super::*;
use crate::project::SourceDevelopment;
pub(super) use crate::source_revision::FileKey;
#[cfg(test)]
use crate::source_revision::FileRevision;
use crate::source_store::{SourceKey as NativeKey, SourceStore};
#[path = "thumbnail_disk.rs"]
mod disk;
#[path = "thumbnail_lru.rs"]
mod lru;
use lru::Cache;
#[path = "editor_embedded_preview.rs"]
mod embedded;
const SOURCE_SIZE: u32 = Proxy::MAX_EDGE;
#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    id: SourceId,
    file: FileKey,
    source: SourceDevelopment,
    development: Development,
}
impl SourceKey {
    fn store_key_at(&self, path: &std::path::Path) -> Option<NativeKey> {
        NativeKey::from_revision(path, self.file.clone(), &self.source, &self.development)
    }
    fn store_key(&self) -> Option<NativeKey> {
        self.store_key_at(&self.file.path)
    }
    fn new(source: &Source, development: &Development, file: FileKey) -> Self {
        Self {
            id: source.id,
            file,
            source: source.development.clone(),
            development: development.clone(),
        }
    }
}
#[derive(Clone, Debug)]
struct FieldKey(Arc<crate::flat_field::FlatField>);
impl PartialEq for FieldKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}
#[derive(Clone, Debug, PartialEq)]
struct ImageKey {
    source: SourceKey,
    target: OutputTarget,
    field: Option<FieldKey>,
}
pub(super) use crate::scan::CameraProxy as Proxy;
impl Proxy {
    pub(super) async fn image(
        &self,
        renderer: &Arc<crate::renderer::GpuRenderer>,
        device: &wgpu::Device,
        target: OutputTarget,
        source: &SourceDevelopment,
        correction: Option<&Arc<Correction>>,
    ) -> Result<Arc<GpuImage>, Error> {
        let mut image = GpuImage::new_shared(
            renderer,
            device,
            self.width,
            self.height,
            &self.pixels,
            self.matrix,
            target,
        )
        .await?;
        if let Some(correction) = correction {
            correction.settings.require_source(source)?;
            image.set_flat_field(
                Some(correction.gpu.clone()),
                crate::gpu::SourceGeometry::new(source.active_area, [self.width, self.height])?,
            )?;
        }
        Ok(Arc::new(image))
    }
}
pub(super) struct SelectedPreview {
    request: u64,
    key: SourceKey,
    field: Option<FieldKey>,
    pub(super) images: [Arc<GpuImage>; 2],
}
impl SelectedPreview {
    pub(super) fn area(&self) -> Rect {
        self.key.source.active_area.into()
    }
    pub(super) fn current(
        &self,
        request: u64,
        document: &Document,
        source: Option<SourceId>,
        correction: Option<&Arc<Correction>>,
    ) -> bool {
        self.request == request
            && source == Some(self.key.id)
            && self.field == correction.map(|c| FieldKey(c.settings.map.clone()))
            && document
                .project
                .source(self.key.id)
                .ok()
                .zip(document.project.development.as_ref())
                .zip(document.source_path(self.key.id).ok())
                .is_some_and(|((source, development), path)| {
                    !source.reference_only
                        && SourceKey::new(source, development, FileKey::new(&path)) == self.key
                })
    }
}
struct Job {
    ticket: u64,
    key: ImageKey,
    path: PathBuf,
    source: Source,
    correction: Option<Arc<Correction>>,
    store: Arc<SourceStore>,
    render: bool,
    priority: development::Priority,
}
struct Outcome {
    ticket: u64,
    key: ImageKey,
    image: Result<Option<Arc<GpuImage>>, String>,
    proper: bool,
}
pub(super) struct Thumbnails {
    cache: Cache<ImageKey, Result<Arc<GpuImage>, String>>,
    pub(super) sources: Arc<SourceStore>,
    selected_key: Option<NativeKey>,
    visible_keys: Vec<NativeKey>,
    // Demand history, not GPU residency or an availability guarantee.
    attempted: Vec<SourceKey>,
    budgets: memory::CacheBudgets,
    pub(super) limits: cache_limits::CacheLimits,
    memory_watch: Option<memory::PressureWatch>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    embedded: embedded::EmbeddedPreviews,
    gpu: Option<(
        wgpu::Device,
        wgpu::Queue,
        Arc<crate::renderer::RendererCache>,
    )>,
    commands: Option<mpsc::Sender<Job>>,
    messages: Option<mpsc::Receiver<Outcome>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    pending: Option<(u64, ImageKey)>,
    pending_render: bool,
    #[cfg(test)]
    pressure_override: Option<memory::Pressure>,
}
impl Default for Thumbnails {
    fn default() -> Self {
        let limits = cache_limits::Hardware::current(None).defaults();
        #[cfg(not(test))]
        let pressure = crate::resources::memory_pressure();
        #[cfg(test)]
        let pressure = memory::Pressure::Normal;
        let budgets = memory::CacheBudgets::new(limits, pressure);
        Self {
            cache: Cache::new(budgets.images, |value| {
                value.as_ref().map_or(1024, |image| {
                    let (source, working) = image.processing_byte_len();
                    (source
                        + working
                        + u64::from(image.linear.width()) * u64::from(image.linear.height()) * 16)
                        as usize
                        + 1024
                })
            }),
            sources: Arc::new(SourceStore::new(budgets.sources as u64)),
            selected_key: None,
            visible_keys: Vec::new(),
            attempted: Vec::new(),
            budgets,
            limits,
            memory_watch: None,
            repaint: Arc::default(),
            embedded: Default::default(),
            gpu: None,
            commands: None,
            messages: None,
            generation: Arc::default(),
            pending: None,
            pending_render: false,
            #[cfg(test)]
            pressure_override: Some(memory::Pressure::Normal),
        }
    }
}
impl Thumbnails {
    pub(super) fn selected_preview(
        &mut self,
        request: u64,
        document: &Document,
        id: SourceId,
        correction: Option<&Arc<Correction>>,
    ) -> Result<Option<SelectedPreview>, Error> {
        let started = std::time::Instant::now();
        let source = document.project.source(id)?;
        let Some(development) = &document.project.development else {
            return Ok(None);
        };
        let key = SourceKey::new(
            source,
            development,
            FileKey::new(&document.source_path(id)?),
        );
        if source.reference_only || key.file.revision.is_none() {
            return Ok(None);
        }
        let Some(native_key) = key.store_key_at(&document.source_path(id)?) else {
            return Ok(None);
        };
        let Some(proxy) = self.sources.get_proxy(&native_key) else {
            return Ok(None);
        };
        let Some((device, queue, renderers)) = &self.gpu else {
            return Ok(None);
        };
        let renderer = renderers.get(device, queue)?;
        // Filmstrip and source-inspection callbacks must not share mutable processing images.
        let preview = SelectedPreview {
            request,
            key: key.clone(),
            field: correction.map(|c| FieldKey(c.settings.map.clone())),
            images: [
                pollster::block_on(proxy.image(
                    &renderer,
                    device,
                    OutputTarget::Srgb,
                    &source.development,
                    correction,
                ))?,
                pollster::block_on(proxy.image(
                    &renderer,
                    device,
                    OutputTarget::DisplayP3,
                    &source.development,
                    correction,
                ))?,
            ],
        };
        if !preview.current(request, document, Some(id), correction) {
            return Ok(None);
        }
        self.selected_key = Some(native_key.clone());
        self.protect_cpu();
        let _ = self.sources.get_proxy(&native_key);
        let bytes: u64 = preview
            .images
            .iter()
            .map(|image| {
                let (source, working) = image.processing_byte_len();
                source
                    + working
                    + u64::from(image.linear.width()) * u64::from(image.linear.height()) * 16
            })
            .sum();
        println!(
            "selected_preview request={request} source_id={} cache=memory dimensions={}x{} gpu_pixel_bytes={bytes} preparation_ms={:.6}",
            id.0,
            proxy.width,
            proxy.height,
            started.elapsed().as_secs_f64() * 1000.
        );
        Ok(Some(preview))
    }
    #[cfg(test)]
    pub fn seed(
        &mut self,
        source: &Source,
        development: &Development,
        file: FileKey,
        proxy: Arc<Proxy>,
    ) {
        let path = file.path.clone();
        self.seed_at(source, development, &path, file, proxy);
    }
    pub(super) fn seed_at(
        &mut self,
        source: &Source,
        development: &Development,
        path: &std::path::Path,
        file: FileKey,
        proxy: Arc<Proxy>,
    ) {
        let source_key = SourceKey::new(source, development, file);
        let Some(key) = source_key.store_key_at(path) else {
            return;
        };
        self.selected_key = Some(key.clone());
        self.protect_cpu();
        // Readmit a live candidate/selected native by-product after the accepted budget is bound.
        let _ = self.sources.get(&key);
        if let Err(error) = self.sources.publish_proxy(key, proxy) {
            eprintln!("Selected proxy cache admission rejected: {error}");
        }
    }
    fn release_selected(&mut self) {
        self.selected_key = None;
        self.protect_cpu();
    }
    pub(super) fn refresh_hidden_memory(&mut self) {
        self.cache.protect(Vec::new());
        self.visible_keys.clear();
        self.protect_cpu();
        self.update_memory();
    }
    fn protect_cpu(&self) {
        let mut keys = self.visible_keys.clone();
        keys.extend(self.selected_key.clone());
        self.sources.protect_proxies(keys);
    }
    pub(super) fn bind_sources(&mut self, sources: Arc<SourceStore>) {
        if !Arc::ptr_eq(&self.sources, &sources) {
            self.cancel();
            self.selected_key = None;
            self.visible_keys.clear();
            self.attempted.clear();
            self.sources = sources;
        }
        self.sources.set_budget(self.budgets.sources as u64);
    }
    pub(super) fn default_limits(&self) -> cache_limits::CacheLimits {
        cache_limits::Hardware::current(self.gpu.as_ref().map(|(device, _, _)| device)).defaults()
    }
    pub(super) fn set_limits(&mut self, limits: cache_limits::CacheLimits) {
        self.limits = limits;
        self.update_memory();
    }
    pub(super) fn update_memory(&mut self) {
        #[cfg(not(test))]
        let pressure = crate::resources::memory_pressure();
        #[cfg(test)]
        let pressure = self
            .pressure_override
            .unwrap_or_else(crate::resources::memory_pressure);
        self.apply_budgets(memory::CacheBudgets::new(self.limits, pressure));
    }
    fn apply_budgets(&mut self, budgets: memory::CacheBudgets) {
        if budgets == self.budgets {
            return;
        }
        self.budgets = budgets;
        if budgets.pressure == memory::Pressure::Critical {
            self.cancel();
        }
        self.cache.set_budget(self.budgets.images);
        self.sources.set_budget(self.budgets.sources as u64);
    }
    fn cancel_offscreen(&mut self, wanted: &[ImageKey]) {
        if let Some((ticket, key)) = &self.pending
            && self.generation.load(std::sync::atomic::Ordering::Relaxed) == *ticket
            && !wanted.contains(key)
        {
            self.cancel();
        }
    }
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        developments: development::Scheduler,
    ) -> Self {
        Self::with_developments(
            device,
            queue,
            renderers,
            disk::DiskCache::default_location(),
            developments,
        )
    }
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn without_disk(&self, developments: development::Scheduler) -> Self {
        let (device, queue, renderers) = self.gpu.as_ref().expect("thumbnail GPU resources");
        Self::with_developments(
            device.clone(),
            queue.clone(),
            renderers.clone(),
            None,
            developments,
        )
    }
    #[cfg(test)]
    fn with_disk(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        disk: Option<disk::DiskCache>,
    ) -> Self {
        Self::with_developments(
            device,
            queue,
            renderers,
            disk,
            development::Scheduler::new(),
        )
    }
    fn with_developments(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        disk: Option<disk::DiskCache>,
        developments: development::Scheduler,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let mut state = Self {
            gpu: Some((device.clone(), queue.clone(), renderers.clone())),
            commands: Some(commands),
            messages: Some(messages),
            ..Default::default()
        };
        state.set_limits(state.default_limits());
        state.embedded = embedded::EmbeddedPreviews::new(state.repaint.clone());
        let generation = state.generation.clone();
        let repaint = state.repaint.clone();
        std::thread::spawn(move || {
            worker(
                device,
                queue,
                renderers,
                receiver,
                send,
                generation,
                disk,
                repaint,
                developments,
            )
        });
        state
    }
    #[cfg(test)]
    pub(super) fn ready_count(&self, document: &Document, target: OutputTarget) -> usize {
        self.cache
            .entries
            .iter()
            .filter(|(key, value)| {
                value.is_ok()
                    && key.target == target
                    && key.field.as_ref().map(|f| &f.0)
                        == document.project.active_flat_field().map(|f| &f.map)
                    && document
                        .project
                        .source(key.source.id)
                        .ok()
                        .zip(document.project.development.as_ref())
                        .is_some_and(|(source, development)| {
                            SourceKey::new(
                                source,
                                development,
                                FileKey::new(
                                    &document.source_path(source.id).expect("thumbnail locator"),
                                ),
                            ) == key.source
                        })
            })
            .count()
    }
    pub fn cancel(&mut self) {
        self.pending = None;
        self.pending_render = false;
        self.embedded.cancel();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn receive(&mut self, document: &Document) {
        loop {
            let outcome = match self.messages.as_ref().map(|messages| messages.try_recv()) {
                Some(Ok(outcome)) => outcome,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    if let Some((_, key)) = self.pending.take() {
                        self.cache
                            .insert(key, Err("Thumbnail worker stopped".into()));
                    }
                    self.commands = None;
                    self.messages = None;
                    break;
                }
                _ => break,
            };
            if self
                .pending
                .as_ref()
                .is_some_and(|(ticket, _)| *ticket == outcome.ticket)
            {
                self.pending = None;
                let current = document
                    .project
                    .source(outcome.key.source.id)
                    .ok()
                    .zip(document.project.development.as_ref())
                    .is_some_and(|(source, development)| {
                        SourceKey::new(
                            source,
                            development,
                            FileKey::new(
                                &document.source_path(source.id).expect("thumbnail locator"),
                            ),
                        ) == outcome.key.source
                            && outcome.key.field.as_ref().map(|f| &f.0)
                                == document.project.active_flat_field().map(|f| &f.map)
                    });
                if self.generation.load(std::sync::atomic::Ordering::Relaxed) == outcome.ticket
                    && current
                {
                    if (outcome.proper || outcome.image.is_err())
                        && !self.attempted.contains(&outcome.key.source)
                    {
                        self.attempted.push(outcome.key.source.clone());
                    }
                    match outcome.image {
                        Ok(Some(image)) => self.cache.insert(outcome.key, Ok(image)),
                        Err(error) => self.cache.insert(outcome.key, Err(error)),
                        Ok(None) => {}
                    }
                }
            }
        }
    }
    #[cfg(test)]
    fn request(
        &mut self,
        key: ImageKey,
        path: PathBuf,
        source: Source,
        correction: Option<Arc<Correction>>,
    ) {
        self.request_mode(key, path, source, correction, true);
    }
    #[cfg(test)]
    fn request_mode(
        &mut self,
        key: ImageKey,
        path: PathBuf,
        source: Source,
        correction: Option<Arc<Correction>>,
        render: bool,
    ) {
        self.request_prioritized(
            key,
            path,
            source,
            correction,
            render,
            if render {
                development::Priority::Visible
            } else {
                development::Priority::Eager
            },
        );
    }
    fn request_prioritized(
        &mut self,
        key: ImageKey,
        path: PathBuf,
        source: Source,
        correction: Option<Arc<Correction>>,
        render: bool,
        priority: development::Priority,
    ) {
        if self.pending.is_some() || self.cache.get(&key).is_some() {
            return;
        }
        let Some(commands) = &self.commands else {
            return;
        };
        let ticket = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if commands
            .send(Job {
                ticket,
                key: key.clone(),
                path,
                source,
                correction,
                store: self.sources.clone(),
                render,
                priority,
            })
            .is_ok()
        {
            self.pending_render = render;
            self.pending = Some((ticket, key));
        } else {
            self.cache
                .insert(key, Err("Thumbnail worker stopped".into()));
        }
    }
}
#[path = "editor_thumbnail_worker.rs"]
mod production_worker;
use production_worker::worker;

#[derive(Default, Debug)]
struct ThumbnailMetrics {
    disk_hit: bool,
    native_hit: bool,
    disk_seconds: f64,
    development_seconds: f64,
    verification_seconds: f64,
    proxy_seconds: f64,
    renderer_seconds: f64,
    image_seconds: f64,
}
fn make_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderers: &crate::renderer::RendererCache,
    job: &Job,
    proxy: &Proxy,
    active: &impl Fn() -> bool,
    metrics: &mut ThumbnailMetrics,
) -> Result<Option<Arc<GpuImage>>, Error> {
    if !active() {
        return Ok(None);
    }
    let tick = std::time::Instant::now();
    let renderer = renderers.get(device, queue)?;
    metrics.renderer_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    let image = pollster::block_on(proxy.image(
        &renderer,
        device,
        job.key.target,
        &job.source.development,
        job.correction.as_ref(),
    ))?;
    metrics.image_seconds = tick.elapsed().as_secs_f64();
    Ok(active().then_some(image))
}

impl Editor {
    fn thumbnail_parameters(&self, source: &Source) -> Result<ProcessingParameters, Error> {
        let photo = if self.current_source == Some(source.id) {
            self.photo
        } else {
            source.photo
        };
        self.calibration().map_or_else(
            || Ok(ProcessingParameters::source()),
            |calibration| {
                self.positive_parameters_for_matrix(
                    calibration,
                    self.roll,
                    photo,
                    glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
                )
            },
        )
    }
    fn thumbnail_work_allowed(&self) -> bool {
        !self.busy
            && self.thumbnails.budgets.pressure != memory::Pressure::Critical
            && self.pick.is_none()
            && self.crop_edit.is_none()
            && self.document_pending.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && self.overwrite.is_none()
            && self.analysis_confirmation.is_none()
    }
    pub(super) fn hidden_filmstrip(&mut self, ui: &egui::Ui) {
        self.thumbnails.refresh_hidden_memory();
        if !self.thumbnail_work_allowed()
            || self.thumbnails.budgets.pressure != memory::Pressure::Normal
        {
            return;
        }
        if let Some(development) = &self.document.project.development {
            for id in &self.document.project.order {
                let source = self.document.project.source(*id).expect("roll source");
                let path = self.document.source_path(*id).expect("roll locator");
                let key = ImageKey {
                    source: SourceKey::new(source, development, FileKey::new(&path)),
                    target: self.target,
                    field: self
                        .correction
                        .as_ref()
                        .map(|c| FieldKey(c.settings.map.clone())),
                };
                if !self.thumbnails.attempted.contains(&key.source)
                    && !self.thumbnails.cache.contains(&key)
                {
                    self.thumbnails.request_prioritized(
                        key,
                        path,
                        source.clone(),
                        self.correction.clone(),
                        false,
                        development::Priority::Eager,
                    );
                    break;
                }
            }
        }
        if self.thumbnails.pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
    pub(super) fn filmstrip(&mut self, ui: &mut egui::Ui) {
        self.thumbnails.bind_sources(self.document.cache().clone());
        if self.thumbnails.memory_watch.is_none() {
            self.thumbnails.repaint.get_or_init(|| ui.ctx().clone());
            self.thumbnails.memory_watch = Some(memory::PressureWatch::new(ui.ctx().clone()));
        }
        self.thumbnails.update_memory();
        self.filmstrip_with_budgets(ui, self.thumbnails.budgets);
    }
    fn filmstrip_with_budgets(&mut self, ui: &mut egui::Ui, budgets: memory::CacheBudgets) {
        if self.ready.is_none() && self.pending_preview.is_none() && self.flat_edit.is_none() {
            self.thumbnails.release_selected();
        }
        let field = self
            .correction
            .as_ref()
            .map(|c| FieldKey(c.settings.map.clone()));
        self.thumbnails.cache.retain(|key, _| key.field == field);
        self.thumbnails.apply_budgets(budgets);
        self.thumbnails.embedded.receive(&self.document, ui.ctx());
        let visible_allowed = self.thumbnail_work_allowed();
        let idle = visible_allowed
            && (self.settled.is_some()
                || self.refinement_error
                    == Some(self.generation.load(std::sync::atomic::Ordering::Relaxed)))
            && self.refining.is_none()
            && self.last_edit.elapsed() >= Duration::from_millis(250);
        let max_height = (ui.ctx().content_rect().height() * 0.35).clamp(52., 240.);
        let mut protected = Vec::new();
        let mut wanted = Vec::new();
        let mut camera_wanted = Vec::new();
        let mut visible = Vec::new();
        let mut select = None;
        let selected_loading = self.ready.is_none()
            && self.selection_timing.is_some()
            && self.document_pending.is_none()
            && self.pick.is_none();
        let selection_enabled = (!self.busy || selected_loading)
            && self.flat_edit.is_none()
            && self.crop_edit.is_none();
        let height = self
            .preferences
            .filmstrip_height
            .min(max_height)
            .min(ui.available_height());
        let panel_id = egui::Id::new("thumbnail-filmstrip");
        if let Some(mut state) = egui::containers::panel::PanelState::load(ui.ctx(), panel_id) {
            // Window constraints must not replace the user's preferred height.
            state.outer_rect.min.y = state.outer_rect.max.y - height;
            ui.ctx()
                .data_mut(|data| data.insert_persisted(panel_id, state));
        }
        let panel = egui::Panel::bottom("thumbnail-filmstrip")
            .resizable(true)
            .default_size(height)
            .min_size(52.)
            .max_size(max_height)
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .fill(egui::Color32::from_gray(22)),
            );
        let strip = panel.show(ui, |ui| {
            ui.take_available_height();
            let layout = ContactSheetLayout::new(ui.available_height(), height < 90.);
            let pitch = layout.pitch();
            let count = self.document.project.order.len();
            egui::ScrollArea::horizontal().show_viewport(ui,|ui,viewport| {
                let width = (count as f32*pitch).max(ui.available_width());
                let (content,_) = ui.allocate_exact_size(egui::vec2(width,layout.size().y),egui::Sense::hover());
                let first = (viewport.min.x/pitch).floor().max(0.) as usize;
                let last = (viewport.max.x/pitch).ceil().max(0.) as usize;
                let last = last.min(count);
                for index in first.min(count)..last {
                    visible.push(index);
                    let id = self.document.project.order[index];
                    let source = self.document.project.source(id).expect("filmstrip source");
                    let path = self.document.source_path(id).expect("filmstrip locator");
                    let selected = self.current_source == Some(id);
                    let response_id = ui.id().with(id.0);
                    let card = layout.begin(ui,content.min+egui::vec2(index as f32*pitch,0.),response_id,selected,selection_enabled);
                    let image_rect = card.image_rect;
                    let key = self.document.project.development.as_ref().map(|development|ImageKey { source:SourceKey::new(source,development,FileKey::new(&path)),target:self.target,field:self.correction.as_ref().map(|c|FieldKey(c.settings.map.clone())) });
                    if let Some(key) = &key { protected.push(key.clone()); }
                    let entry = key.as_ref().and_then(|key|self.thumbnails.cache.get(key)).cloned();
                    let parameters = self.thumbnail_parameters(source);
                    let mut error = None;
                    match (entry,parameters) {
                        (Some(Ok(image)),Ok(parameters)) => {
                            let photo_orientation = if self.current_source == Some(source.id) { self.photo_orientation } else { source.orientation };
                            let crop = if self.current_source == Some(source.id) { self.crop } else { source.crop };
                            let composition = Composition::with_crop(source.development.active_area.into(), self.roll_orientation, photo_orientation, crop)
                                .expect("document source has a valid active area");
                            let dimensions = composition.size();
                            let aspect = dimensions[0] as f32/dimensions[1] as f32;
                            let size = egui::vec2((image_rect.height()*aspect).min(image_rect.width()),(image_rect.width()/aspect).min(image_rect.height()));
                            let full = egui::Rect::from_center_size(image_rect.center(), size);
                            let draw = full.intersect(ui.clip_rect());
                            // egui-wgpu clamps offscreen viewports; remap clipped, pixel-rounded bounds instead of squeezing the whole image.
                            let scale = ui.ctx().pixels_per_point();
                            let min = (full.min.to_vec2() * scale).round();
                            let extent = (full.max.to_vec2() * scale).round() - min;
                            let draw_min = (draw.min.to_vec2() * scale).round();
                            let draw_max = (draw.max.to_vec2() * scale).round();
                            if draw_max.x > draw_min.x && draw_max.y > draw_min.y {
                                let frame_point = |point: egui::Vec2| {
                                    let fraction = (point - min) / extent;
                                    crate::geometry::FramePoint([f64::from(fraction.x) * f64::from(dimensions[0]), f64::from(fraction.y) * f64::from(dimensions[1])])
                                };
                                let mapping = composition.texture_mapping(frame_point(draw_min), frame_point(draw_max), composition.area());
                                ui.painter().with_clip_rect(draw).add(egui_wgpu::Callback::new_paint_callback(draw,ImagePaint::new(image,Some(parameters),mapping)));
                            }
                        }
                        (Some(Err(message)),_) => { error = Some(message); }
                        (_,Err(message)) => { error = Some(message.to_string()); }
                        (None,Ok(_)) => {
                            if let Some(key) = key {
                                if let Some(texture) = self.thumbnails.embedded.get(&key.source) {
                                    let dimensions = texture.size_vec2();
                                    let scale = (image_rect.width()/dimensions.x).min(image_rect.height()/dimensions.y);
                                    let rect = egui::Rect::from_center_size(image_rect.center(),dimensions*scale);
                                    let painter = ui.painter().with_clip_rect(image_rect.intersect(ui.clip_rect()));
                                    painter.image(texture.id(),rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.,1.)),egui::Color32::WHITE);
                                    card.camera_badge(ui);
                                } else {
                                    card.loading(ui);
                                }
                                if self.thumbnails.embedded.get(&key.source).is_some()
                                    || !key.source.store_key().is_some_and(|key| self.thumbnails.sources.contains_proxy(&key)) {
                                    camera_wanted.push((index,key.clone(),path.clone(),source.clone()));
                                }
                                wanted.push((index,key,path.clone(),source.clone()));
                            }
                        }
                    }
                    if error.is_some() { card.error(ui); }
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    card.caption(ui,&name);
                    if card.response.clicked() && selection_enabled { select = Some(index); }
                    let help = error.unwrap_or_else(||format!("{}\nCamera previews show embedded, ungraded camera framing until the developed thumbnail is ready. Drag the strip's top edge to resize.",path.display()));
                    card.response.on_hover_text(&help).on_disabled_hover_text(help);
                }
            });
        });
        if (strip.response.rect.height() - height).abs() > 0.5 {
            self.preferences.filmstrip_height = strip.response.rect.height().clamp(52., 240.);
        }
        let mut eligible = protected.clone();
        // Finish each source even at zero optional GPU retention. Offscreen GPU work
        // needs spare space and no foreground refinement demand; CPU/disk work does not.
        let eager_allowed =
            visible_allowed && self.thumbnails.budgets.pressure == memory::Pressure::Normal;
        let render_offscreen = idle && self.thumbnails.cache.can_retain(256 * 256 * 40 + 1024);
        if eager_allowed
            && let Some(development) = &self.document.project.development
            && let (Some(first), Some(last)) = (visible.first().copied(), visible.last().copied())
        {
            let count = self.document.project.order.len();
            for index in (0..count).filter(|index| *index < first || *index > last) {
                let source = self
                    .document
                    .project
                    .source(self.document.project.order[index])
                    .expect("prefetch source");
                let path = self
                    .document
                    .source_path(source.id)
                    .expect("prefetch locator");
                let key = ImageKey {
                    source: SourceKey::new(source, development, FileKey::new(&path)),
                    target: self.target,
                    field: self
                        .correction
                        .as_ref()
                        .map(|c| FieldKey(c.settings.map.clone())),
                };
                eligible.push(key.clone());
                if !self.thumbnails.attempted.contains(&key.source)
                    && !self.thumbnails.cache.contains(&key)
                {
                    wanted.push((index, key, path, source.clone()));
                }
            }
        }
        if !eager_allowed
            || (!idle && self.thumbnails.pending_render)
            || wanted
                .iter()
                .any(|(index, _, _, _)| visible.contains(index))
        {
            self.thumbnails.cancel_offscreen(&protected);
        } else {
            self.thumbnails.cancel_offscreen(&eligible);
        }
        self.thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        self.thumbnails.protect_cpu();
        self.thumbnails.cache.protect(protected);
        self.thumbnails
            .embedded
            .request_visible(&camera_wanted, visible_allowed && select.is_none());
        if let Some(index) = select {
            let id = self.document.project.order[index];
            let path = self
                .document
                .source_path(id)
                .expect("selected filmstrip locator");
            if matches!(self.pick, Some(Pick::Dmin | Pick::Dmax)) {
                self.pick_roll_source(id);
            } else {
                self.selected = index;
                self.load(path);
            }
        } else if visible_allowed {
            wanted.sort_by_key(|(index, _, _, _)| {
                let distance = visible
                    .iter()
                    .map(|visible| index.abs_diff(*visible))
                    .min()
                    .unwrap_or(0);
                (
                    !visible.contains(index),
                    distance,
                    index.abs_diff(self.selected),
                )
            });
            if let Some((index, key, path, source)) = wanted.into_iter().next() {
                self.thumbnails.request_prioritized(
                    key,
                    path,
                    source,
                    self.correction.clone(),
                    visible.contains(&index) || render_offscreen,
                    if visible.contains(&index) {
                        development::Priority::Visible
                    } else {
                        development::Priority::Eager
                    },
                );
            }
        }
        if self.thumbnails.pending.is_some() || self.thumbnails.embedded.pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
#[path = "editor_thumbnail_clipping_tests.rs"]
mod clipping_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "../native/cache_observation.rs"]
mod observation;
#[cfg(test)]
#[path = "editor_renderer_tests.rs"]
mod renderer_tests;
#[cfg(test)]
#[path = "editor_source_revision_tests.rs"]
mod revision_tests;
#[cfg(test)]
#[path = "editor_selected_preview_tests.rs"]
mod selected_tests;
#[cfg(test)]
#[path = "editor_thumbnail_selection_tests.rs"]
mod selection_tests;
#[cfg(test)]
#[path = "editor_source_cache_tests.rs"]
mod source_cache_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "../native/editor_switching_observation.rs"]
mod switch_observation;
#[cfg(test)]
#[path = "editor_thumbnails_tests.rs"]
mod tests;
