use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

struct Request {
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

pub(super) struct Finished {
    identity: Arc<AtomicBool>,
    key: SourceKey,
    image: Completion,
}

#[cfg(test)]
struct Work {
    request: Request,
    cancelled: Arc<AtomicBool>,
    reply: futures_channel::oneshot::Sender<Result<Option<egui::ColorImage>, String>>,
}

async fn load_preview(request: Request, cancelled: Arc<AtomicBool>) -> Completion {
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

#[cfg(test)]
async fn load_controlled(
    commands: mpsc::Sender<Work>,
    request: Request,
    cancelled: Arc<AtomicBool>,
) -> Completion {
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

struct Pending {
    cancelled: Arc<AtomicBool>,
    _task: Option<async_executor::Task<()>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

// Closing the executor drops the future before it drops the preview owner.
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
    enabled: bool,
    #[cfg(test)]
    controlled_load: Option<mpsc::Sender<Work>>,
    pending: Option<Pending>,
    visible: Vec<SourceKey>,
    budget: usize,
}

impl EmbeddedPreviews {
    pub(super) fn for_document(&self) -> Self {
        Self {
            enabled: self.enabled,
            #[cfg(test)]
            controlled_load: self.controlled_load.clone(),
            ..Self::default()
        }
    }
}

impl Default for EmbeddedPreviews {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            enabled: false,
            #[cfg(test)]
            controlled_load: None,
            pending: None,
            visible: Vec::new(),
            budget: 16 << 20,
        }
    }
}

impl EmbeddedPreviews {
    pub(super) fn new() -> Self {
        Self {
            enabled: true,
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

    pub(super) fn start<'scope, T: 'scope>(
        &mut self,
        document: &Document,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
        initialization: &crate::source::Initialization,
        context: &egui::Context,
        cx: &UiContext<'scope, T>,
        publish: impl FnOnce(UiContext<'scope, T>, Finished) + 'scope,
    ) {
        let Some((request, cancelled)) = self.begin(document, sources, initialization) else {
            return;
        };

        let key = request.key.clone();
        let fallback_key = key.clone();
        let identity = cancelled.clone();
        let retire = Retire(cancelled.clone());
        #[cfg(test)]
        let controlled = self.controlled_load.clone();
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            #[cfg(test)]
            let image = if let Some(commands) = controlled {
                load_controlled(commands, request, identity.clone()).await
            } else {
                load_preview(request, identity.clone()).await
            };
            #[cfg(not(test))]
            let image = load_preview(request, identity.clone()).await;

            publish(
                cx,
                Finished {
                    identity,
                    key,
                    image,
                },
            );
        });

        if let Some(task) = task {
            self.pending.as_mut().expect("admitted preview")._task = Some(task);
        } else {
            self.complete(
                Finished {
                    identity: cancelled,
                    key: fallback_key,
                    image: Completion::Stopped,
                },
                document,
                context,
            );
        }
    }

    fn begin(
        &mut self,
        document: &Document,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
        initialization: &crate::source::Initialization,
    ) -> Option<(Request, Arc<AtomicBool>)> {
        if self.pending.is_some() || !self.enabled {
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
        let cancelled = Arc::new(AtomicBool::new(false));
        self.pending = Some(Pending {
            cancelled: cancelled.clone(),
            _task: None,
        });
        Some((
            Request {
                key,
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

    /// Consume a current completion, even if cancellation or changed inputs prevent publication.
    pub(super) fn complete(
        &mut self,
        finished: Finished,
        document: &Document,
        context: &egui::Context,
    ) -> bool {
        let Finished {
            identity,
            key,
            image,
        } = finished;
        if !self
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(&pending.cancelled, &identity))
        {
            return false;
        }
        let pending = self.pending.take().unwrap();
        if matches!(&image, Completion::Stopped) {
            self.enabled = false;
        }
        if pending.cancelled.load(Ordering::Relaxed) || !current(&key, document) {
            return true;
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
        true
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
