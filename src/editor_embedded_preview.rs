use super::*;
use image::{ImageDecoder, metadata::Orientation};

#[cfg(test)]
#[path = "editor_embedded_preview_tests.rs"]
mod tests;

struct Request {
    ticket: u64,
    key: SourceKey,
    path: PathBuf,
    source: Source,
}
struct ResultMessage {
    ticket: u64,
    key: SourceKey,
    image: Result<Option<egui::ColorImage>, String>,
}
#[derive(Default)]
pub(super) struct EmbeddedPreviews {
    entries: Vec<(SourceKey, Option<egui::TextureHandle>)>,
    commands: Option<mpsc::Sender<Request>>,
    messages: Option<mpsc::Receiver<ResultMessage>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    pending: Option<(u64, SourceKey)>,
}
impl EmbeddedPreviews {
    pub(super) fn new(repaint: Arc<std::sync::OnceLock<egui::Context>>) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let state = Self {
            commands: Some(commands),
            messages: Some(messages),
            ..Default::default()
        };
        let generation = state.generation.clone();
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let started = std::time::Instant::now();
                let active =
                    || generation.load(std::sync::atomic::Ordering::Relaxed) == request.ticket;
                let image = load(&request, &active);
                println!(
                    "embedded_preview source_id={} available={} failed={} cancelled={} seconds={:.6}",
                    request.key.id.0,
                    matches!(&image, Ok(Some(_))),
                    image.is_err(),
                    !active(),
                    started.elapsed().as_secs_f64()
                );
                if send
                    .send(ResultMessage {
                        ticket: request.ticket,
                        key: request.key,
                        image,
                    })
                    .is_err()
                {
                    break;
                }
                if let Some(context) = repaint.get() {
                    context.request_repaint();
                }
            }
            if let Some(context) = repaint.get() {
                context.request_repaint();
            }
        });
        state
    }
    pub(super) fn cancel(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub(super) fn receive(&mut self, document: &Document, context: &egui::Context) {
        loop {
            let message = match self.messages.as_ref().map(|messages| messages.try_recv()) {
                Some(Ok(message)) => message,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    if let Some((_, key)) = self.pending.take() {
                        self.entries.push((key, None));
                    }
                    self.commands = None;
                    self.messages = None;
                    break;
                }
                _ => break,
            };
            if !self
                .pending
                .as_ref()
                .is_some_and(|(ticket, _)| *ticket == message.ticket)
            {
                continue;
            }
            self.pending = None;
            if self.generation.load(std::sync::atomic::Ordering::Relaxed) != message.ticket
                || !current(&message.key, document)
            {
                continue;
            }
            let texture = match message.image {
                Ok(Some(image)) => Some(context.load_texture(
                    "embedded camera preview",
                    image,
                    egui::TextureOptions::LINEAR,
                )),
                Ok(None) => None,
                Err(reason) => {
                    eprintln!(
                        "embedded_preview unavailable source_id={} reason={reason}",
                        message.key.id.0
                    );
                    None
                }
            };
            self.entries.push((message.key, texture));
        }
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
        // Retain only missing visible cells, never recent/offscreen JPEGs or a second disk cache.
        self.entries
            .retain(|(key, _)| wanted.iter().any(|(_, image, _, _)| &image.source == key));
        if let Some((ticket, key)) = &self.pending
            && self.generation.load(std::sync::atomic::Ordering::Relaxed) == *ticket
            && (!allowed || !wanted.iter().any(|(_, image, _, _)| &image.source == key))
        {
            self.cancel();
        }
        if !allowed || self.pending.is_some() {
            return;
        }
        let Some((_, key, path, source)) = wanted.iter().find(|(_, key, _, _)| {
            !self
                .entries
                .iter()
                .any(|(candidate, _)| candidate == &key.source)
        }) else {
            return;
        };
        let Some(commands) = &self.commands else {
            return;
        };
        let ticket = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if commands
            .send(Request {
                ticket,
                key: key.source.clone(),
                path: path.clone(),
                source: source.clone(),
            })
            .is_ok()
        {
            self.pending = Some((ticket, key.source.clone()));
        } else {
            self.entries.push((key.source.clone(), None));
        }
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
    let (actual, mut raw) = document::open_source(&request.path).map_err(|e| e.to_string())?;
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
