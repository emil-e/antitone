use super::*;
use image::{ImageDecoder, metadata::Orientation};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
#[path = "editor_embedded_preview_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "editor_embedded_preview_workflow_tests.rs"]
mod workflow_tests;

struct Request {
    ticket: u64,
    key: SourceKey,
    path: PathBuf,
    source: Source,
}

enum Completion {
    Image(Result<Option<egui::ColorImage>, String>),
    Stopped,
}

struct Work {
    request: Request,
    cancelled: Arc<AtomicBool>,
    reply: futures_channel::oneshot::Sender<Result<Option<egui::ColorImage>, String>>,
    // Last to drop: producer admission includes native/decode and captured-payload cleanup.
    _admission: async_lock::SemaphoreGuardArc,
}

#[derive(Clone)]
struct Services {
    commands: mpsc::Sender<Work>,
    admission: Arc<async_lock::Semaphore>,
}

impl Services {
    fn new() -> Self {
        let (commands, receiver) = mpsc::channel::<Work>();
        std::thread::spawn(move || {
            while let Ok(work) = receiver.recv() {
                let started = std::time::Instant::now();
                let active = || !work.cancelled.load(Ordering::Relaxed);
                let image = load(&work.request, &active);
                println!(
                    "embedded_preview source_id={} available={} failed={} cancelled={} seconds={:.6}",
                    work.request.key.id.0,
                    matches!(&image, Ok(Some(_))),
                    image.is_err(),
                    !active(),
                    started.elapsed().as_secs_f64()
                );
                // A departed waiter cannot stop this shared resource worker.
                let _ = work.reply.send(image);
            }
        });
        Self {
            commands,
            admission: Arc::new(async_lock::Semaphore::new(1)),
        }
    }

    async fn load(self, request: Request, cancelled: Arc<AtomicBool>) -> Completion {
        let admission = self.admission.acquire_arc().await;
        let (reply, result) = futures_channel::oneshot::channel();
        if self
            .commands
            .send(Work {
                request,
                cancelled,
                reply,
                _admission: admission,
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

#[derive(Default)]
pub(super) struct EmbeddedPreviews {
    entries: Vec<(SourceKey, Option<egui::TextureHandle>)>,
    services: Option<Services>,
    next: u64,
    pending: Option<Projection>,
    wanted: Vec<(SourceKey, PathBuf)>,
    allowed: bool,
}

impl EmbeddedPreviews {
    pub(super) fn new() -> Self {
        Self {
            services: Some(Services::new()),
            ..Default::default()
        }
    }

    pub(super) fn cancel(&mut self) {
        self.allowed = false;
        if let Some(pending) = &self.pending {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn reset_demand(&mut self) {
        self.cancel();
        self.pending = None;
        self.wanted.clear();
    }

    fn begin(&mut self, document: &Document) -> Option<(Request, Arc<AtomicBool>)> {
        if !self.allowed || self.pending.is_some() || self.services.is_none() {
            return None;
        }
        let (key, path) = self.wanted.iter().find(|(key, _)| {
            current(key, document) && !self.entries.iter().any(|(candidate, _)| candidate == key)
        })?;
        let source = document.project.source(key.id).ok()?.clone();
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
                eprintln!(
                    "embedded_preview unavailable source_id={} reason={reason}",
                    key.id.0
                );
                None
            }
        };
        self.entries.push((key, texture));
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
        // Keep only visible missing cells; source records are captured one at a time.
        self.entries
            .retain(|(key, _)| wanted.iter().any(|(_, image, _, _)| &image.source == key));
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
    }

    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl Editor {
    pub(in crate::editor) fn start_embedded_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
        context: &egui::Context,
    ) {
        if self.fullscreen.is_some() || !self.thumbnail_work_allowed() {
            self.thumbnails.embedded.cancel();
            return;
        }
        if self.document.embedded_preview.is_some() {
            return;
        }
        let Some((request, cancelled)) = self.thumbnails.embedded.begin(&self.document) else {
            return;
        };
        let ticket = request.ticket;
        let key = request.key.clone();
        let fallback_key = key.clone();
        let services = self.thumbnails.embedded.services.clone().unwrap();
        let retire = Retire(cancelled.clone());
        let stopped = cancelled.clone();
        let target = access.clone();
        let workflow_context = context.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let image = services.load(request, stopped).await;
            target.update(|editor| {
                if !editor
                    .document
                    .embedded_preview
                    .as_ref()
                    .is_some_and(|pending| pending.ticket == ticket)
                {
                    return;
                }
                let pending = editor.document.embedded_preview.take().unwrap();
                editor.thumbnails.embedded.complete(
                    ticket,
                    key,
                    image,
                    &editor.document,
                    &workflow_context,
                );
                drop(pending);
                editor.start_embedded_workflow(&target, &workflow_context);
            });
        }) else {
            self.thumbnails.embedded.complete(
                ticket,
                fallback_key,
                Completion::Stopped,
                &self.document,
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
    !document.cache().is_closed()
        && document
            .project
            .source(key.id)
            .ok()
            .zip(document.project.development.as_ref())
            .zip(document.source_path(key.id).ok())
            .is_some_and(|((source, development), path)| {
                !source.reference_only
                    && SourceKey::new(source, development, FileKey::new(&path)) == *key
            })
}

fn load(request: &Request, active: &impl Fn() -> bool) -> Result<Option<egui::ColorImage>, String> {
    if !active() || FileKey::new(&request.path) != request.key.file {
        return Ok(None);
    }
    let (actual, mut raw) =
        document::open_source(&request.path, Some(request.key.development.fixed_wb))
            .map_err(|e| e.to_string())?;
    document::verify_source(&request.source, &actual).map_err(|e| e.to_string())?;
    let Some(thumbnail) = raw.embedded_thumbnail().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    if !active() {
        return Ok(None);
    }
    let image = decode(thumbnail)?;
    if !active() || FileKey::new(&request.path) != request.key.file {
        return Ok(None);
    }
    Ok(image)
}

fn decode(thumbnail: crate::raw::EmbeddedThumbnail) -> Result<Option<egui::ColorImage>, String> {
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(thumbnail.jpeg))
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let profile = decoder.icc_profile().map_err(|e| e.to_string())?;
    if profile.is_none() && thumbnail.color_space != 1 {
        return Ok(None);
    }
    let orientation = decoder
        .exif_metadata()
        .map_err(|e| e.to_string())?
        .as_deref()
        .and_then(Orientation::from_exif_chunk);
    let orientation = match orientation {
        Some(orientation) => orientation,
        None => match thumbnail.flip {
            0 => Orientation::NoTransforms,
            3 => Orientation::Rotate180,
            5 => Orientation::Rotate270,
            6 => Orientation::Rotate90,
            _ => return Ok(None),
        },
    };
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    if image.width() > SOURCE_SIZE || image.height() > SOURCE_SIZE {
        image = image.thumbnail(SOURCE_SIZE, SOURCE_SIZE);
    }
    let mut image = image.into_rgb8();
    if let Some(profile) = profile {
        let input = lcms2::Profile::new_icc(&profile).map_err(|e| e.to_string())?;
        let output = lcms2::Profile::new_srgb();
        let transform = lcms2::Transform::<u8, u8>::new(
            &input,
            lcms2::PixelFormat::RGB_8,
            &output,
            lcms2::PixelFormat::RGB_8,
            lcms2::Intent::RelativeColorimetric,
        )
        .map_err(|e| e.to_string())?;
        transform.transform_in_place(image.as_mut());
    }
    Ok(Some(egui::ColorImage::from_rgb(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )))
}
