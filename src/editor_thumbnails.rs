use super::*;
use crate::project::{Fingerprint, SourceDevelopment};
use std::collections::VecDeque;

const CAPACITY: usize = 16;
const SOURCE_SIZE: u32 = 256;
#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    id: SourceId,
    fingerprint: Fingerprint,
    source: SourceDevelopment,
    development: Development,
}
impl SourceKey {
    fn new(source: &Source, development: &Development) -> Self {
        Self {
            id: source.id,
            fingerprint: source.fingerprint.clone(),
            source: source.development.clone(),
            development: development.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
struct ImageKey {
    source: SourceKey,
    target: OutputTarget,
}
struct Cache<K, V> {
    entries: VecDeque<(K, V)>,
}
impl<K, V> Default for Cache<K, V> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }
}
impl<K: PartialEq, V> Cache<K, V> {
    fn get(&mut self, key: &K) -> Option<&V> {
        let index = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = self.entries.remove(index).unwrap();
        self.entries.push_back(entry);
        self.entries.back().map(|(_, value)| value)
    }
    fn insert(&mut self, key: K, value: V) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == &key)
        {
            self.entries.remove(index);
        }
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }
}
struct Proxy {
    width: u32,
    height: u32,
    pixels: Vec<[u16; 4]>,
    matrix: glam::DMat3,
}
struct Job {
    ticket: u64,
    key: ImageKey,
    path: PathBuf,
    source: Source,
}
struct Outcome {
    ticket: u64,
    key: ImageKey,
    image: Result<Option<Arc<GpuImage>>, String>,
}
#[derive(Default)]
pub(super) struct Thumbnails {
    cache: Cache<ImageKey, Result<Arc<GpuImage>, String>>,
    commands: Option<mpsc::Sender<Job>>,
    messages: Option<mpsc::Receiver<Outcome>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    pending: Option<(u64, ImageKey)>,
}
impl Thumbnails {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let state = Self {
            commands: Some(commands),
            messages: Some(messages),
            ..Default::default()
        };
        let generation = state.generation.clone();
        std::thread::spawn(move || worker(device, queue, receiver, send, generation));
        state
    }
    pub(super) fn ready_count(&self, document: &Document, target: OutputTarget) -> usize {
        self.cache
            .entries
            .iter()
            .filter(|(key, value)| {
                value.is_ok()
                    && key.target == target
                    && document
                        .project
                        .source(key.source.id)
                        .ok()
                        .zip(document.project.development.as_ref())
                        .is_some_and(|(source, development)| {
                            SourceKey::new(source, development) == key.source
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
                        SourceKey::new(source, development) == outcome.key.source
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
    fn request(&mut self, key: ImageKey, path: PathBuf, source: Source) {
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
fn worker(
    device: wgpu::Device,
    queue: wgpu::Queue,
    commands: mpsc::Receiver<Job>,
    messages: mpsc::Sender<Outcome>,
    generation: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut sources: Cache<SourceKey, Arc<Proxy>> = Cache::default();
    while let Ok(job) = commands.recv() {
        let started = std::time::Instant::now();
        let cached = sources.get(&job.key.source).cloned();
        let cache_hit = cached.is_some();
        let active = || generation.load(std::sync::atomic::Ordering::Relaxed) == job.ticket;
        let image = (|| -> Result<Option<Arc<GpuImage>>, Error> {
            if !active() {
                return Ok(None);
            }
            let proxy = match cached {
                Some(proxy) => proxy,
                None => {
                    let (actual, _) = document::read_source(&job.path)?;
                    document::verify_source(&job.source, &actual)?;
                    if !active() {
                        return Ok(None);
                    }
                    let scan = Scan::open(&job.path, Some(job.key.source.development.fixed_wb))?;
                    document::verify_scan(&actual, &scan)?;
                    let (width, height, pixels) = scan.preview_source(SOURCE_SIZE)?;
                    let proxy = Arc::new(Proxy {
                        width,
                        height,
                        pixels,
                        matrix: scan.camera_to_ap0,
                    });
                    sources.insert(job.key.source.clone(), proxy.clone());
                    proxy
                }
            };
            if !active() {
                return Ok(None);
            }
            let image = pollster::block_on(GpuImage::new(
                &device,
                &queue,
                proxy.width,
                proxy.height,
                &proxy.pixels,
                proxy.matrix,
                job.key.target,
            ))?;
            Ok(active().then(|| Arc::new(image)))
        })()
        .map_err(|error| error.to_string());
        if matches!(image, Ok(Some(_))) {
            println!(
                "thumbnail source_id={} target={:?} source_cache_hit={} seconds={:.3}",
                job.key.source.id.0,
                job.key.target,
                cache_hit,
                started.elapsed().as_secs_f64()
            );
        }
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

impl Editor {
    fn thumbnail_parameters(&self, source: &Source) -> Result<ProcessingParameters, Error> {
        let photo = if self.current_source == Some(source.id) {
            self.photo
        } else {
            source.photo
        };
        self.calibration().map_or_else(
            || Ok(ProcessingParameters::source()),
            |calibration| ProcessingParameters::positive(calibration, self.roll, photo),
        )
    }
    pub(super) fn filmstrip(&mut self, ui: &mut egui::Ui) {
        let max_height = (ui.ctx().content_rect().height() * 0.35).clamp(52., 240.);
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
            // Keep the visible page within cache capacity even when the strip is very short.
            let image_width = (image_height*1.5).max(ui.available_width()/(CAPACITY-1) as f32-8.);
            let pitch = image_width+8.;
            let count = self.document.project.order.len();
            egui::ScrollArea::horizontal().show_viewport(ui,|ui,viewport| {
                let width = (count as f32*pitch).max(ui.available_width());
                let (content,_) = ui.allocate_exact_size(egui::vec2(width,image_height+22.),egui::Sense::hover());
                let first = (viewport.min.x/pitch).floor().max(0.) as usize;
                let last = ((viewport.max.x/pitch).ceil() as usize+1).min(count);
                for index in first.min(count)..last {
                    visible.push(index);
                    let id = self.document.project.order[index];
                    let source = self.document.project.source(id).expect("filmstrip source");
                    let path = self.document.source_path(id).expect("filmstrip locator");
                    let cell = egui::Rect::from_min_size(content.min+egui::vec2(index as f32*pitch,0.),egui::vec2(image_width,image_height+22.));
                    let response = ui.interact(cell,ui.id().with(id.0),egui::Sense::click());
                    let selected = self.ready.is_some() && self.current_source == Some(id);
                    let visuals = ui.style().interact_selectable(&response,selected);
                    ui.painter().rect_filled(cell,3.,visuals.bg_fill);
                    let image_rect = egui::Rect::from_min_size(cell.min,egui::vec2(image_width,image_height)).shrink(2.);
                    let key = self.document.project.development.as_ref().map(|development|ImageKey { source:SourceKey::new(source,development),target:self.target });
                    let entry = key.as_ref().and_then(|key|self.thumbnails.cache.get(key)).cloned();
                    let parameters = self.thumbnail_parameters(source);
                    let mut error = None;
                    match (entry,parameters) {
                        (Some(Ok(image)),Ok(parameters)) => {
                            let aspect = image.linear.width() as f32/image.linear.height() as f32;
                            let size = egui::vec2((image_rect.height()*aspect).min(image_rect.width()),(image_rect.width()/aspect).min(image_rect.height()));
                            ui.painter().with_clip_rect(ui.clip_rect()).add(egui_wgpu::Callback::new_paint_callback(egui::Rect::from_center_size(image_rect.center(),size),ImagePaint::new(image,Some(parameters),[0.,0.,1.,1.])));
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
                    if response.clicked() && !self.busy { select = Some(index); }
                    response.on_hover_text(error.unwrap_or_else(||format!("{}\nApproximate whole-frame thumbnail; drag the strip's top edge to resize.",path.display())));
                }
            });
        });
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
        } else if !self.busy
            && self.pick.is_none()
            && self.document_pending.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && self.overwrite.is_none()
            && self.settled.is_some()
            && self.refining.is_none()
            && self.last_edit.elapsed() >= Duration::from_millis(250)
        {
            visible.sort_by_key(|index| index.abs_diff(self.selected));
            visible.truncate(CAPACITY);
            wanted.retain(|(index, _, _, _)| visible.contains(index));
            wanted.sort_by_key(|(index, _, _, _)| index.abs_diff(self.selected));
            if let Some((_, key, path, source)) = wanted.into_iter().next() {
                self.thumbnails.request(key, path, source);
            }
        }
        if self.thumbnails.pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
#[path = "editor_thumbnails_tests.rs"]
mod tests;
