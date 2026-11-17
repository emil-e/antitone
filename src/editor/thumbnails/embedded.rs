use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

struct Request {
    ticket: u64,
    key: SourceKey,
    #[cfg(test)]
    path: PathBuf,
    #[cfg(test)]
    source: Source,
    physical: crate::source::Input,
    sources: Option<Arc<dyn crate::source::SourceProvider>>,
}

enum Completion {
    Image(Result<Option<egui::ColorImage>, String>),
    Stopped,
}

#[cfg(test)]
struct Work {
    request: Request,
    cancelled: Arc<AtomicBool>,
    reply: futures_channel::oneshot::Sender<Result<Option<egui::ColorImage>, String>>,
}

#[derive(Clone)]
enum Services {
    Production,
    #[cfg(test)]
    Controlled(mpsc::Sender<Work>),
}

impl Services {
    async fn load(self, request: Request, cancelled: Arc<AtomicBool>) -> Completion {
        match self {
            #[cfg(test)]
            Self::Controlled(commands) => {
                let (reply, result) = futures_channel::oneshot::channel();
                if commands
                    .send(Work {
                        request,
                        cancelled,
                        reply,
                    })
                    .is_err()
                {
                    return Completion::Stopped;
                }
                match result.await {
                    Ok(image) => Completion::Image(image),
                    Err(_) => Completion::Stopped,
                }
            }
            Self::Production => {
                if cancelled.load(Ordering::Relaxed) {
                    return Completion::Image(Ok(None));
                }
                let Some(sources) = request.sources else {
                    return Completion::Stopped;
                };
                let preview = request.physical.preview(sources.as_ref()).await;
                match crate::background::run(move |_| {
                    if cancelled.load(Ordering::Relaxed) {
                        return Ok(None);
                    }
                    preview.and_then(|preview| preview.as_deref().map(colour_image).transpose())
                })
                .await
                {
                    Ok(image) => Completion::Image(image),
                    Err(_) => Completion::Stopped,
                }
            }
        }
    }
}

struct Projection {
    ticket: u64,
    cancelled: Arc<AtomicBool>,
}

pub(in crate::editor) struct Pending {
    ticket: u64,
    cancelled: Arc<AtomicBool>,
    _task: async_executor::Task<()>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

enum CameraImage {
    Unavailable,
    Ready(egui::TextureHandle),
    // Keep completion and measured size when optional pixels leave; retry when they fit.
    Evicted { bytes: usize },
}

impl CameraImage {
    fn texture(&self) -> Option<&egui::TextureHandle> {
        match self {
            Self::Ready(texture) => Some(texture),
            Self::Unavailable | Self::Evicted { .. } => None,
        }
    }
}

pub(super) struct EmbeddedPreviews {
    entries: Vec<(SourceKey, CameraImage)>,
    services: Option<Services>,
    next: u64,
    pending: Option<Projection>,
    visible: Vec<SourceKey>,
    budget: usize,
}

impl EmbeddedPreviews {
    pub(super) fn for_document(&self) -> Self {
        Self {
            services: self.services.clone(),
            ..Self::default()
        }
    }
}

impl Default for EmbeddedPreviews {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            services: None,
            next: 0,
            pending: None,
            visible: Vec::new(),
            budget: 16 << 20,
        }
    }
}

impl EmbeddedPreviews {
    pub(super) fn new() -> Self {
        Self {
            services: Some(Services::Production),
            ..Default::default()
        }
    }

    pub(super) fn cancel(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn entry_bytes(key: &SourceKey, image: &CameraImage) -> usize {
        let Some(texture) = image.texture() else {
            return 0;
        };
        let [width, height] = texture.size();
        std::mem::size_of::<SourceKey>()
            + key.file.path.capacity()
            + key.source.camera_model.capacity()
            + key.source.decode.version().len()
            + 1024
            + width * height * 4
    }

    pub(super) fn byte_len(&self) -> usize {
        self.entries
            .iter()
            .map(|(key, texture)| Self::entry_bytes(key, texture))
            .sum()
    }

    pub(super) fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.trim();
    }

    fn trim(&mut self) {
        let mut bytes = self.byte_len();
        while bytes > self.budget {
            let Some(index) = self
                .entries
                .iter()
                .position(|(key, image)| image.texture().is_some() && !self.visible.contains(key))
            else {
                break;
            };
            let (key, image) = &mut self.entries[index];
            let released = Self::entry_bytes(key, image);
            *image = CameraImage::Evicted { bytes: released };
            bytes = bytes.saturating_sub(released);
        }
    }

    fn begin(
        &mut self,
        document: &Document,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
        initialization: &crate::source::Initialization,
    ) -> Option<(Request, Arc<AtomicBool>)> {
        if self.pending.is_some() || self.services.is_none() {
            return None;
        }
        self.entries.retain(|(key, _)| current(key, document));
        let mut candidates = document.project.order.clone();
        candidates.sort_by_key(|id| !self.visible.iter().any(|key| key.id == *id));
        let retained = self.byte_len();
        let (key, _path) = candidates.into_iter().find_map(|id| {
            let source = document.project.source(id).ok()?;
            let path = document.source_path(id).ok()?;
            let key = SourceKey::for_document(source, document.file_key(&path), document);
            match self.entries.iter().find(|(candidate, _)| candidate == &key) {
                None => Some((key, path)),
                Some((_, CameraImage::Evicted { bytes }))
                    if self.visible.contains(&key)
                        || retained.saturating_add(*bytes) <= self.budget =>
                {
                    Some((key, path))
                }
                Some(_) => None,
            }
        })?;
        let physical = document.source_input(key.id, initialization).ok()?;
        #[cfg(test)]
        let source = key.metadata_record(&_path);
        self.next += 1;
        let ticket = self.next;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.pending = Some(Projection {
            ticket,
            cancelled: cancelled.clone(),
        });
        Some((
            Request {
                ticket,
                key: key.clone(),
                #[cfg(test)]
                path: _path,
                #[cfg(test)]
                source,
                physical,
                sources,
            },
            cancelled,
        ))
    }

    fn complete(
        &mut self,
        ticket: u64,
        key: SourceKey,
        image: Completion,
        document: &Document,
        context: &egui::Context,
    ) {
        if !self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.ticket == ticket)
        {
            return;
        }
        let pending = self.pending.take().unwrap();
        if matches!(&image, Completion::Stopped) {
            self.services = None;
        }
        if pending.cancelled.load(Ordering::Relaxed) || !current(&key, document) {
            return;
        }
        let texture = match image {
            Completion::Image(Ok(Some(image))) => Some(context.load_texture(
                "embedded camera preview",
                image,
                egui::TextureOptions::LINEAR,
            )),
            Completion::Image(Ok(None)) | Completion::Stopped => None,
            Completion::Image(Err(reason)) => {
                log::debug!(
                    source_id = key.id.0,
                    error = reason.as_str();
                    "embedded_preview_unavailable"
                );
                None
            }
        };
        self.entries.retain(|(candidate, _)| candidate != &key);
        self.entries.push((
            key,
            texture.map_or(CameraImage::Unavailable, CameraImage::Ready),
        ));
        self.trim();
    }

    pub(super) fn get(&self, key: &SourceKey) -> Option<&egui::TextureHandle> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate == key)
            .and_then(|(_, image)| image.texture())
    }

    pub(super) fn request_visible(&mut self, wanted: &[(usize, ImageKey, PathBuf)]) {
        // Touch visible entries without discarding recently seen navigation feedback.
        for (_, image, _) in wanted {
            if let Some(index) = self
                .entries
                .iter()
                .position(|(key, _)| key == &image.source)
            {
                let entry = self.entries.remove(index);
                self.entries.push(entry);
            }
        }
        self.visible = wanted
            .iter()
            .map(|(_, image, _)| image.source.clone())
            .collect();
        self.trim();
    }

    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl Editor {
    pub(super) fn request_embedded_previews(
        &mut self,
        wanted: &[(usize, ImageKey, PathBuf)],
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) {
        self.document.thumbnails.embedded.request_visible(wanted);
        self.document.thumbnails.sync_image_budget();
        self.acquire_embedded(context, cx);
    }

    pub(super) fn acquire_embedded(&mut self, context: &egui::Context, cx: &UiContext<'_, Self>) {
        if self.document.thumbnails.budgets.pressure == memory::Pressure::Critical {
            self.document.thumbnails.embedded.cancel();
            return;
        }
        if self.document.embedded_preview.is_some() {
            return;
        }
        let sources = self.document.thumbnails.acquisition();
        let Some((request, cancelled)) = self.document.thumbnails.embedded.begin(
            &self.document.state,
            sources,
            &self.document.initialization,
        ) else {
            return;
        };
        let ticket = request.ticket;
        let key = request.key.clone();
        let fallback_key = key.clone();
        let services = self.document.thumbnails.embedded.services.clone().unwrap();
        let retire = Retire(cancelled.clone());
        let stopped = cancelled.clone();
        let workflow_context = context.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let image = services.load(request, stopped).await;
            cx.update(|editor| {
                if !editor
                    .document
                    .embedded_preview
                    .as_ref()
                    .is_some_and(|pending| pending.ticket == ticket)
                {
                    return;
                }
                let pending = editor.document.embedded_preview.take().unwrap();
                editor.document.thumbnails.embedded.complete(
                    ticket,
                    key,
                    image,
                    &editor.document.state,
                    &workflow_context,
                );
                drop(pending);
                editor.document.thumbnails.update_memory();
                editor.document.thumbnails.sync_image_budget();
                editor.acquire_embedded(&workflow_context, &cx);
            });
        }) else {
            self.document.thumbnails.embedded.complete(
                ticket,
                fallback_key,
                Completion::Stopped,
                &self.document.state,
                context,
            );
            return;
        };
        self.document.embedded_preview = Some(Pending {
            ticket,
            cancelled,
            _task: task,
        });
    }
}

fn current(key: &SourceKey, document: &Document) -> bool {
    document
        .project
        .source(key.id)
        .ok()
        .zip(document.source_path(key.id).ok())
        .is_some_and(|(source, path)| {
            SourceKey::for_document(source, document.file_key(&path), document) == *key
        })
}

#[cfg(test)]
fn load(request: &Request, active: &impl Fn() -> bool) -> Result<Option<egui::ColorImage>, String> {
    if !active() {
        return Ok(None);
    }

    let preview = match &request.sources {
        Some(sources) => pollster::block_on(request.physical.preview(sources.as_ref())),
        None => crate::embedded_preview::read(&request.path, &request.source.development),
    }?;

    if !active() {
        return Ok(None);
    }

    preview.as_deref().map(colour_image).transpose()
}

fn colour_image(preview: &crate::embedded_preview::Preview) -> Result<egui::ColorImage, String> {
    let rgb = preview.srgb()?;
    Ok(egui::ColorImage::from_rgb(
        [rgb.width() as usize, rgb.height() as usize],
        rgb.as_raw(),
    ))
}

#[cfg(test)]
fn decode(thumbnail: crate::raw::EmbeddedThumbnail) -> Result<Option<egui::ColorImage>, String> {
    crate::embedded_preview::decode(thumbnail)?
        .as_ref()
        .map(colour_image)
        .transpose()
}
