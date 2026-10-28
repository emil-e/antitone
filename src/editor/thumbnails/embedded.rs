use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

struct Request {
    ticket: u64,
    key: SourceKey,
    path: PathBuf,
    source: Source,
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
                let key = crate::source::SourceKey::for_load(
                    &request.path,
                    request.key.file,
                    &request.source.development,
                );
                let preview = sources.load_preview(key).await;
                match crate::background::run(move || {
                    if cancelled.load(Ordering::Relaxed) {
                        return Ok(None);
                    }
                    preview.map(|preview| preview.map(|preview| colour_image(&preview)))
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
    key: SourceKey,
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

pub(super) struct EmbeddedPreviews {
    entries: Vec<(SourceKey, Option<egui::TextureHandle>)>,
    services: Option<Services>,
    next: u64,
    pending: Option<Projection>,
    wanted: Vec<(SourceKey, PathBuf)>,
    allowed: bool,
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
            wanted: Vec::new(),
            allowed: false,
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
        self.allowed = false;
        if let Some(pending) = &self.pending {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn entry_bytes(key: &SourceKey, texture: &Option<egui::TextureHandle>) -> usize {
        std::mem::size_of::<SourceKey>()
            + key.file.path.capacity()
            + key.source.camera_model.capacity()
            + key
                .source
                .lens_correction
                .as_ref()
                .map_or(0, String::capacity)
            + key.source.decode.backend_version.capacity()
            + key.source.decode.contract.capacity()
            + key.source.decode.inversion_space.capacity()
            + key.source.decode.working_space.capacity()
            + 1024
            + texture.as_ref().map_or(0, |texture| {
                let [width, height] = texture.size();
                width * height * 4
            })
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
                .position(|(key, _)| !self.wanted.iter().any(|(wanted, _)| wanted == key))
            else {
                break;
            };
            let (key, texture) = self.entries.remove(index);
            bytes = bytes.saturating_sub(Self::entry_bytes(&key, &texture));
        }
    }

    fn begin(
        &mut self,
        document: &Document,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
    ) -> Option<(Request, Arc<AtomicBool>)> {
        if !self.allowed || self.pending.is_some() || self.services.is_none() {
            return None;
        }
        let (key, path) = self.wanted.iter().find(|(key, _)| {
            current(key, document) && !self.entries.iter().any(|(candidate, _)| candidate == key)
        })?;
        let source = key.metadata_record(path);
        self.next += 1;
        let ticket = self.next;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.pending = Some(Projection {
            ticket,
            key: key.clone(),
            cancelled: cancelled.clone(),
        });
        Some((
            Request {
                ticket,
                key: key.clone(),
                path: path.clone(),
                source,
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
        self.entries.push((key, texture));
        self.trim();
    }

    pub(super) fn get(&self, key: &SourceKey) -> Option<&egui::TextureHandle> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate == key)
            .and_then(|(_, image)| image.as_ref())
    }

    pub(super) fn request_visible(
        &mut self,
        wanted: &[(usize, ImageKey, PathBuf, Source)],
        allowed: bool,
    ) {
        // Touch visible entries without discarding recently seen navigation feedback.
        for (_, image, _, _) in wanted {
            if let Some(index) = self
                .entries
                .iter()
                .position(|(key, _)| key == &image.source)
            {
                let entry = self.entries.remove(index);
                self.entries.push(entry);
            }
        }
        if let Some(pending) = &self.pending
            && (!allowed
                || !wanted
                    .iter()
                    .any(|(_, image, _, _)| image.source == pending.key))
        {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
        self.allowed = allowed;
        self.wanted = wanted
            .iter()
            .map(|(_, image, path, _)| (image.source.clone(), path.clone()))
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
        wanted: &[(usize, ImageKey, PathBuf, Source)],
        allowed: bool,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) {
        self.document
            .thumbnails
            .embedded
            .request_visible(wanted, allowed);
        self.document.thumbnails.sync_image_budget();
        self.acquire_embedded(context, cx);
    }

    fn acquire_embedded(&mut self, context: &egui::Context, cx: &UiContext<'_, Self>) {
        if self.fullscreen.is_some() || !self.thumbnail_work_allowed() {
            self.document.thumbnails.embedded.cancel();
            return;
        }
        if self.document.embedded_preview.is_some() {
            return;
        }
        let sources = self.document.thumbnails.acquisition();
        let Some((request, cancelled)) = self
            .document
            .thumbnails
            .embedded
            .begin(&self.document.state, sources)
        else {
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
            !source.reference_only && SourceKey::new(source, document.file_key(&path)) == *key
        })
}

#[cfg(test)]
fn load(request: &Request, active: &impl Fn() -> bool) -> Result<Option<egui::ColorImage>, String> {
    if !active() {
        return Ok(None);
    }

    let preview = match &request.sources {
        Some(sources) => {
            pollster::block_on(sources.load_preview(crate::source::SourceKey::for_load(
                &request.path,
                request.key.file.clone(),
                &request.source.development,
            )))
        }
        None => crate::embedded_preview::read(&request.path, &request.source.development),
    }?;

    if !active() {
        return Ok(None);
    }

    Ok(preview.map(|preview| colour_image(&preview)))
}

fn colour_image(preview: &crate::embedded_preview::Preview) -> egui::ColorImage {
    egui::ColorImage::from_rgb(
        [preview.rgb.width() as usize, preview.rgb.height() as usize],
        preview.rgb.as_raw(),
    )
}

#[cfg(test)]
fn decode(thumbnail: crate::raw::EmbeddedThumbnail) -> Result<Option<egui::ColorImage>, String> {
    crate::embedded_preview::decode(thumbnail)
        .map(|preview| preview.map(|preview| colour_image(&preview)))
}
