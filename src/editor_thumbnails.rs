use super::*;
use crate::project::SourceDevelopment;
use serde::{Deserialize, Serialize};
#[path = "thumbnail_disk.rs"]
mod disk;
#[path = "thumbnail_lru.rs"]
mod lru;
use lru::Cache;
const SOURCE_SIZE: u32 = 256;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRevision {
    bytes: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
}
impl FileRevision {
    fn read(path: &std::path::Path) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileKey {
    path: PathBuf,
    revision: Option<FileRevision>,
}
impl FileKey {
    pub(super) fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
            // Unavailable stats disable disk reuse; opening/development reports source errors.
            revision: FileRevision::read(path).ok(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    id: SourceId,
    file: FileKey,
    source: SourceDevelopment,
    development: Development,
}
impl SourceKey {
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
pub(super) struct Proxy {
    width: u32,
    height: u32,
    pixels: Vec<[u16; 4]>,
    matrix: glam::DMat3,
}
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
    pub(super) fn from_scan(scan: &Scan) -> Result<Self, Error> {
        let (width, height, pixels) = scan.preview_source(SOURCE_SIZE)?;
        Ok(Self {
            width,
            height,
            pixels,
            matrix: scan.camera_to_ap1,
        })
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
}
struct Outcome {
    ticket: u64,
    key: ImageKey,
    image: Result<Option<Arc<GpuImage>>, String>,
}
pub(super) struct Thumbnails {
    cache: Cache<ImageKey, Result<Arc<GpuImage>, String>>,
    sources: Arc<std::sync::Mutex<Cache<SourceKey, Arc<Proxy>>>>,
    budgets: memory::CacheBudgets,
    memory_watch: Option<memory::PressureWatch>,
    gpu: Option<(
        wgpu::Device,
        wgpu::Queue,
        Arc<crate::renderer::RendererCache>,
    )>,
    commands: Option<mpsc::Sender<Job>>,
    messages: Option<mpsc::Receiver<Outcome>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    pending: Option<(u64, ImageKey)>,
}
impl Default for Thumbnails {
    fn default() -> Self {
        let budgets = memory::CacheBudgets::current();
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
            sources: Arc::new(std::sync::Mutex::new(Cache::new(
                budgets.sources,
                |proxy: &Arc<Proxy>| proxy.pixels.len() * 8 + 1024,
            ))),
            budgets,
            memory_watch: None,
            gpu: None,
            commands: None,
            messages: None,
            generation: Arc::default(),
            pending: None,
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
        let Some(proxy) = self.sources.lock().unwrap().get(&key).cloned() else {
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
        self.sources.lock().unwrap().protect(vec![key]);
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
    pub fn seed(
        &mut self,
        source: &Source,
        development: &Development,
        file: FileKey,
        proxy: Arc<Proxy>,
    ) {
        let key = SourceKey::new(source, development, file);
        let mut sources = self.sources.lock().unwrap();
        sources.protect(vec![key.clone()]);
        sources.insert(key, proxy);
    }
    fn release_selected(&mut self) {
        self.sources.lock().unwrap().protect(Vec::new());
    }
    fn update_memory(&mut self) {
        self.apply_budgets(memory::CacheBudgets::current());
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
        self.sources
            .lock()
            .unwrap()
            .set_budget(self.budgets.sources);
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
    ) -> Self {
        Self::with_disk(
            device,
            queue,
            renderers,
            disk::DiskCache::default_location(),
        )
    }
    fn with_disk(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        disk: Option<disk::DiskCache>,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let state = Self {
            gpu: Some((device.clone(), queue.clone(), renderers.clone())),
            commands: Some(commands),
            messages: Some(messages),
            ..Default::default()
        };
        let generation = state.generation.clone();
        let sources = state.sources.clone();
        std::thread::spawn(move || {
            worker(
                device, queue, renderers, sources, receiver, send, generation, disk,
            )
        });
        state
    }
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
                    match outcome.image {
                        Ok(Some(image)) => self.cache.insert(outcome.key, Ok(image)),
                        Err(error) => self.cache.insert(outcome.key, Err(error)),
                        Ok(None) => {}
                    }
                }
            }
        }
    }
    fn request(
        &mut self,
        key: ImageKey,
        path: PathBuf,
        source: Source,
        correction: Option<Arc<Correction>>,
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
            })
            .is_ok()
        {
            self.pending = Some((ticket, key));
        } else {
            self.cache
                .insert(key, Err("Thumbnail worker stopped".into()));
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn worker(
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderers: Arc<crate::renderer::RendererCache>,
    sources: Arc<std::sync::Mutex<Cache<SourceKey, Arc<Proxy>>>>,
    commands: mpsc::Receiver<Job>,
    messages: mpsc::Sender<Outcome>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    disk: Option<disk::DiskCache>,
) {
    while let Ok(job) = commands.recv() {
        let started = std::time::Instant::now();
        let cached = sources.lock().unwrap().get(&job.key.source).cloned();
        let cache_hit = cached.is_some();
        let mut metrics = ThumbnailMetrics::default();
        let active = || generation.load(std::sync::atomic::Ordering::Relaxed) == job.ticket;
        let image = (|| -> Result<Option<Arc<GpuImage>>, Error> {
            if !active() || FileKey::new(&job.path) != job.key.source.file {
                return Ok(None);
            }
            let proxy = match cached {
                Some(proxy) => {
                    if let Some(disk) = &disk
                        && let Err(error) = disk.store_if_missing(&job.key.source, &proxy)
                    {
                        eprintln!(
                            "thumbnail disk_cache_write_failed source_id={} reason={error}",
                            job.source.id.0
                        );
                    }
                    proxy
                }
                None => {
                    if let Some(disk) = &disk {
                        let tick = std::time::Instant::now();
                        let cached = disk.load(&job.key.source);
                        metrics.disk_seconds = tick.elapsed().as_secs_f64();
                        match cached {
                            Ok(Some(proxy)) => {
                                metrics.disk_hit = true;
                                sources
                                    .lock()
                                    .unwrap()
                                    .insert(job.key.source.clone(), proxy.clone());
                                return make_image(
                                    &device,
                                    &queue,
                                    &renderers,
                                    &job,
                                    &proxy,
                                    &active,
                                    &mut metrics,
                                );
                            }
                            Ok(None) => {}
                            Err(error) => eprintln!(
                                "thumbnail disk_cache_miss source_id={} reason={error}",
                                job.source.id.0
                            ),
                        }
                    }
                    let tick = std::time::Instant::now();
                    let scan = Scan::open_with_priority(
                        &job.path,
                        Some(job.key.source.development.fixed_wb),
                        crate::raw::DevelopmentPriority::Background,
                    )?;
                    metrics.development_seconds = tick.elapsed().as_secs_f64();
                    let tick = std::time::Instant::now();
                    document::verify_scan(&job.source, &scan)?;
                    metrics.verification_seconds = tick.elapsed().as_secs_f64();
                    let tick = std::time::Instant::now();
                    let proxy = Arc::new(Proxy::from_scan(&scan)?);
                    metrics.proxy_seconds = tick.elapsed().as_secs_f64();
                    sources
                        .lock()
                        .unwrap()
                        .insert(job.key.source.clone(), proxy.clone());
                    if let Some(disk) = &disk
                        && let Err(error) = disk.store(&job.key.source, &proxy)
                    {
                        eprintln!(
                            "thumbnail disk_cache_write_failed source_id={} reason={error}",
                            job.source.id.0
                        );
                    }
                    proxy
                }
            };
            if !active() {
                return Ok(None);
            }
            make_image(
                &device,
                &queue,
                &renderers,
                &job,
                &proxy,
                &active,
                &mut metrics,
            )
        })()
        .map_err(|error| error.to_string());
        println!(
            "thumbnail source_id={} target={:?} source_cache_hit={} disk_cache_hit={} cancelled={} failed={} seconds={:.6} phases={metrics:?}",
            job.key.source.id.0,
            job.key.target,
            cache_hit,
            metrics.disk_hit,
            matches!(image, Ok(None)),
            image.is_err(),
            started.elapsed().as_secs_f64()
        );
        if messages
            .send(Outcome {
                ticket: job.ticket,
                key: job.key,
                image,
            })
            .is_err()
        {
            break;
        }
    }
}

#[derive(Default, Debug)]
struct ThumbnailMetrics {
    disk_hit: bool,
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
    pub(super) fn filmstrip(&mut self, ui: &mut egui::Ui) {
        if self.thumbnails.memory_watch.is_none() {
            self.thumbnails.memory_watch = Some(memory::PressureWatch::new(ui.ctx().clone()));
        }
        if self.ready.is_none() && self.pending_preview.is_none() && self.flat_edit.is_none() {
            self.thumbnails.release_selected();
        }
        let field = self
            .correction
            .as_ref()
            .map(|c| FieldKey(c.settings.map.clone()));
        self.thumbnails.cache.retain(|key, _| key.field == field);
        self.thumbnails.update_memory();
        let idle = !self.busy
            && self.thumbnails.budgets.pressure != memory::Pressure::Critical
            && self.pick.is_none()
            && self.crop_edit.is_none()
            && self.document_pending.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && self.overwrite.is_none()
            && (self.settled.is_some()
                || self.refinement_error
                    == Some(self.generation.load(std::sync::atomic::Ordering::Relaxed)))
            && self.refining.is_none()
            && self.last_edit.elapsed() >= Duration::from_millis(250);
        let max_height = (ui.ctx().content_rect().height() * 0.35).clamp(52., 240.);
        let mut protected = Vec::new();
        let mut wanted = Vec::new();
        let mut visible = Vec::new();
        let mut select = None;
        let panel = egui::Panel::bottom("thumbnail-filmstrip")
            .resizable(true)
            .default_size(140.)
            .min_size(52.)
            .max_size(max_height);
        panel.show(ui, |ui| {
            ui.take_available_height();
            let image_height = (ui.available_height()-28.).max(20.);
            let image_width = (image_height*1.5).max(64.);
            let pitch = image_width+8.;
            let count = self.document.project.order.len();
            egui::ScrollArea::horizontal().show_viewport(ui,|ui,viewport| {
                let width = (count as f32*pitch).max(ui.available_width());
                let (content,_) = ui.allocate_exact_size(egui::vec2(width,image_height+22.),egui::Sense::hover());
                let first = (viewport.min.x/pitch).floor().max(0.) as usize;
                let last = (viewport.max.x/pitch).ceil().max(0.) as usize;
                let last = last.min(count);
                for index in first.min(count)..last {
                    visible.push(index);
                    let id = self.document.project.order[index];
                    let source = self.document.project.source(id).expect("filmstrip source");
                    let path = self.document.source_path(id).expect("filmstrip locator");
                    let cell = egui::Rect::from_min_size(content.min+egui::vec2(index as f32*pitch,0.),egui::vec2(image_width,image_height+22.));
                    let response = ui.interact(cell,ui.id().with(id.0),egui::Sense::click());
                    let selected = (self.ready.is_some() || self.pending_preview.is_some()) && self.current_source == Some(id);
                    let visuals = ui.style().interact_selectable(&response,selected);
                    ui.painter().rect_filled(cell,3.,visuals.bg_fill);
                    let image_rect = egui::Rect::from_min_size(cell.min,egui::vec2(image_width,image_height)).shrink(2.);
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
                            let mapping = composition.texture_mapping(crate::geometry::FramePoint([0.,0.]), crate::geometry::FramePoint(dimensions.map(f64::from)), composition.area());
                            ui.painter().with_clip_rect(ui.clip_rect()).add(egui_wgpu::Callback::new_paint_callback(egui::Rect::from_center_size(image_rect.center(),size),ImagePaint::new(image,Some(parameters),mapping)));
                        }
                        (Some(Err(message)),_) => { error = Some(message); }
                        (_,Err(message)) => { error = Some(message.to_string()); }
                        (None,Ok(_)) => {
                            if let Some(key) = key { wanted.push((index,key,path.clone(),source.clone())); }
                            ui.painter().text(image_rect.center(),egui::Align2::CENTER_CENTER,"…",egui::FontId::proportional(16.),visuals.text_color());
                        }
                    }
                    if error.is_some() { ui.painter().text(image_rect.center(),egui::Align2::CENTER_CENTER,"!",egui::FontId::proportional(16.),visuals.text_color()); }
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let galley = ui.painter().layout(name.into_owned(),egui::FontId::proportional(11.),visuals.text_color(),image_width);
                    ui.painter().with_clip_rect(cell.intersect(ui.clip_rect())).galley(cell.min+egui::vec2(2.,image_height+2.),galley,visuals.text_color());
                    if response.clicked() && !self.busy && self.flat_edit.is_none() && self.crop_edit.is_none() { select = Some(index); }
                    response.on_hover_text(error.unwrap_or_else(||format!("{}\nApproximate whole-frame thumbnail; drag the strip's top edge to resize.",path.display())));
                }
            });
        });
        let mut eligible = protected.clone();
        // Idle roll prefetch never evicts retained recent/visible work to make room.
        if self.thumbnails.budgets.pressure == memory::Pressure::Normal
            && self.thumbnails.cache.can_retain(256 * 256 * 40 + 1024)
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
                if !self.thumbnails.cache.contains(&key) {
                    wanted.push((index, key, path, source.clone()));
                }
            }
        }
        if !idle
            || wanted
                .iter()
                .any(|(index, _, _, _)| visible.contains(index))
        {
            self.thumbnails.cancel_offscreen(&protected);
        } else {
            self.thumbnails.cancel_offscreen(&eligible);
        }
        self.thumbnails.cache.protect(protected);
        if let Some(index) = select {
            let id = self.document.project.order[index];
            let path = self
                .document
                .source_path(id)
                .expect("selected filmstrip locator");
            if matches!(self.pick, Some(Pick::Dmin | Pick::Dmax)) {
                self.pick_source(path);
            } else {
                self.selected = index;
                self.load(path);
            }
        } else if idle {
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
            if let Some((_, key, path, source)) = wanted.into_iter().next() {
                self.thumbnails
                    .request(key, path, source, self.correction.clone());
            }
        }
        if self.thumbnails.pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
#[path = "editor_performance_tests.rs"]
mod performance;
#[cfg(test)]
#[path = "editor_selected_preview_tests.rs"]
mod selected_tests;
#[cfg(test)]
#[path = "editor_thumbnails_tests.rs"]
mod tests;
