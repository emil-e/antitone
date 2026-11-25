//! Processed thumbnails: image retention, preparation and worker execution.
#[cfg(test)]
use crate::source::FileRevision;
use crate::{
    Error,
    processing::InputSettings,
    project::{Source, SourceDevelopment, SourceId},
    rendering::OutputTarget,
    source::{self as source_loading, FileKey, SourceKey as NativeKey},
    viewer::GpuImage,
};
use std::{sync::Arc, time::Duration};
pub(crate) mod disk;
pub(crate) mod lru;
mod worker;
use lru::Cache;
pub(crate) use worker::{Job, Outcome, Services, Stopped, ThumbnailGpu, Worker};
#[cfg(test)]
pub(crate) use worker::{Submission, ThumbnailMetrics};
const SOURCE_SIZE: u32 = Proxy::MAX_EDGE;
pub(crate) const REFRESH_QUIET_PERIOD: Duration = Duration::from_millis(150);

pub(crate) struct Thumbnail {
    pub image: Arc<GpuImage>,
    pub graded: bool,
    pub current: bool,
}

impl Processed {
    pub(crate) fn image(&self, key: &ImageKey) -> Option<Result<Thumbnail, String>> {
        let (product, current) = self.image_product(key)?;
        Some(product.map(|product| Thumbnail {
            image: product.image.clone(),
            graded: product.positive.is_some(),
            current,
        }))
    }

    fn image_product(&self, key: &ImageKey) -> Option<(CachedProduct, bool)> {
        if let Some(product) = self.cache.peek(key) {
            return Some((product.clone(), true));
        }
        // Keep a previous grade while the current request prepares.
        self.previous_product(key)
            .map(|(_, product)| Ok(product.clone()))
            .or_else(|| {
                self.cache.entries.iter().rev().find_map(|(old, value)| {
                    (key.same_presentation(old) && value.is_err()).then(|| value.clone())
                })
            })
            .map(|product| (product, false))
    }
}

pub(crate) enum Demand {
    Skip,
    Wait,
    Request { render: bool },
}

pub(crate) struct Processed {
    pub(crate) cache: Cache<ImageKey, CachedProduct>,
    pub(crate) attempted_sources: Vec<SourceKey>,
    pub(crate) gpu: Option<ThumbnailGpu>,
    pub(crate) services: Option<Services>,
    pub(crate) generation: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) pending: Option<(u64, ImageKey)>,
    refresh: Option<(ImageKey, std::time::Instant)>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    pub(crate) work: ThumbnailWork,
}

impl Processed {
    pub(crate) fn new(budget: usize, repaint: Arc<std::sync::OnceLock<egui::Context>>) -> Self {
        Self {
            cache: Cache::new(budget, |key: &ImageKey, value: &CachedProduct| {
                key.retained_bytes()
                    - key
                        .luts()
                        .assets()
                        .map(|asset| asset.numeric_bytes())
                        .sum::<usize>()
                    + match value {
                        Ok(product) => {
                            product.byte_len()
                                - product
                                    .look_resources()
                                    .iter()
                                    .map(|lut| lut.byte_len() as usize)
                                    .sum::<usize>()
                        }
                        Err(reason) => 1024 + reason.capacity(),
                    }
            })
            .with_shared_weight(shared_look_bytes),
            attempted_sources: Vec::new(),
            gpu: None,
            services: None,
            generation: Arc::default(),
            pending: None,
            refresh: None,
            repaint,
            work: ThumbnailWork::default(),
        }
    }

    pub(crate) fn for_document(&self, budget: usize) -> Self {
        Self {
            gpu: self.gpu.clone(),
            services: self.services.clone(),
            ..Self::new(budget, self.repaint.clone())
        }
    }

    pub(crate) fn request(
        &mut self,
        key: ImageKey,
        physical: source_loading::Input,
        correction: crate::photographic::stages::flat_field::resources::Resources,
        render: bool,
        priority: source_loading::Priority,
        sources: Option<Arc<dyn source_loading::SourceProvider>>,
    ) -> Option<(
        impl Future<Output = Result<Outcome, Stopped>> + use<>,
        Dispatch,
    )> {
        if self.pending.is_some() || self.cache.get(&key).is_some() {
            return None;
        }
        let Some(services) = self.services.clone() else {
            self.mark_unavailable(std::slice::from_ref(&key));
            return None;
        };
        let generation = self.generation.clone();
        let ticket = generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let previous = render
            .then(|| {
                self.source_product(&key)
                    .map(|(_, product)| product.clone())
            })
            .flatten();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        let job = Job {
            generation: generation.clone(),
            cancelled,
            reference: physical.reference_exposure(),
            ticket,
            key: key.clone(),
            physical,
            correction,
            render,
            priority,
            previous,
        };
        self.pending = Some((ticket, key.clone()));
        Some((
            async move { services.process(job, sources).await },
            Dispatch {
                id: Ticket { ticket, generation },
                key,
                retire,
            },
        ))
    }

    pub(crate) fn track(&mut self, dispatch: Dispatch, task: async_executor::Task<()>) {
        self.work.tasks.push(Pending {
            id: dispatch.id,
            _task: task,
            _retire: dispatch.retire,
        });
    }

    pub(crate) fn take_work(&mut self, id: &Ticket) -> Option<Pending> {
        if !Arc::ptr_eq(&id.generation, &self.generation) {
            return None;
        }
        let index = self
            .work
            .tasks
            .iter()
            .position(|pending| pending.id.ticket == id.ticket)?;
        Some(self.work.tasks.swap_remove(index))
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn pending_key(&self) -> Option<&ImageKey> {
        self.pending.as_ref().map(|(_, key)| key)
    }

    pub(crate) fn reconcile(
        &mut self,
        current: bool,
        visible: bool,
        eager: bool,
        wanted: &[ImageKey],
    ) {
        if !current || (!visible && (!eager || wanted.iter().any(|key| !self.cache.contains(key))))
        {
            self.cancel();
        }
    }

    pub(crate) fn next(
        &mut self,
        key: &ImageKey,
        visible: bool,
        continuing: bool,
        now: std::time::Instant,
    ) -> Demand {
        if self.cache.contains(key) {
            return Demand::Skip;
        }
        let render = visible || self.can_retain(key);
        if !render && self.attempted_sources.contains(&key.source) {
            return Demand::Skip;
        }
        // Keep a compatible result while edits settle. After publishing the first
        // new result, drain the queue without a quiet period per image.
        if !continuing
            && render
            && self.previous_product(key).is_some()
            && self.defer_refresh(key, now)
        {
            return Demand::Wait;
        }
        self.clear_refresh();
        Demand::Request { render }
    }

    pub(crate) fn clear_refresh(&mut self) {
        self.refresh = None;
    }

    pub(crate) fn mark_unavailable(&mut self, wanted: &[ImageKey]) -> bool {
        if self.services.is_some() {
            return false;
        }
        if self.gpu.is_some() {
            for key in wanted {
                if !self.cache.contains(key) {
                    self.cache
                        .insert(key.clone(), Err("Thumbnail worker stopped".into()));
                }
            }
        }
        true
    }

    pub(crate) fn complete(
        &mut self,
        outcome: Outcome,
        source_current: bool,
        recipe_current: impl FnOnce(&ImageKey) -> bool,
    ) -> bool {
        let matching = self
            .pending
            .as_ref()
            .is_some_and(|(ticket, _)| *ticket == outcome.ticket);
        if matching {
            self.pending = None;
        }
        // The GPU ticket can retire while useful CPU/disk work completes.
        if source_current
            && (outcome.proper || (matching && outcome.image.is_err()))
            && !self.attempted_sources.contains(&outcome.key.source)
        {
            self.attempted_sources.push(outcome.key.source.clone());
        }
        if !matching
            || !source_current
            || self.generation.load(std::sync::atomic::Ordering::Relaxed) != outcome.ticket
            || !recipe_current(&outcome.key)
        {
            return false;
        }
        match outcome.image {
            Ok(Some(image)) => {
                self.cache.retain(|key, _| key.source != outcome.key.source);
                self.cache.insert(outcome.key, Ok(image));
                true
            }
            Err(error) => {
                self.cache.insert(outcome.key, Err(error));
                false
            }
            Ok(None) => false,
        }
    }

    pub(crate) fn can_retain(&self, key: &ImageKey) -> bool {
        let Some(mut extra) = key.estimated_product_bytes() else {
            return false;
        };
        for look in key.luts().assets() {
            if self.cache.entries.iter().any(|(old, _)| {
                old.luts()
                    .assets()
                    .any(|old| old.allocation() == look.allocation())
            }) {
                extra -= look.numeric_bytes();
            }
            if self.cache.entries.iter().any(|(_, product)| {
                product.as_ref().ok().is_some_and(|product| {
                    product
                        .look_resources()
                        .iter()
                        .any(|old| old.id() == look.id())
                })
            }) {
                // The renderer reuses a live immutable GPU LUT by content identity.
                extra -= look.gpu_bytes() as usize;
            }
        }
        self.cache.can_retain(extra)
    }

    pub(crate) fn source_product(&self, key: &ImageKey) -> Option<(&ImageKey, &Arc<Product>)> {
        if !matches!(&key.recipe, Ok(Some(_))) {
            return None;
        }
        self.cache.entries.iter().rev().find_map(|(old, product)| {
            let product = product.as_ref().ok()?;
            (key.source.same_pixels(&old.source) && product.positive.is_some())
                .then_some((old, product))
        })
    }

    pub(crate) fn previous_product(&self, key: &ImageKey) -> Option<(&ImageKey, &Arc<Product>)> {
        self.cache.entries.iter().rev().find_map(|(old, product)| {
            key.same_presentation(old)
                .then(|| product.as_ref().ok().map(|product| (old, product)))
                .flatten()
        })
    }

    pub(crate) fn set_budget(&mut self, bytes: usize) -> usize {
        self.cache.set_budget(bytes);
        self.cache.measured_bytes()
    }

    pub(crate) fn retain_field(&mut self, field: Option<FieldKey>) {
        self.cache.retain(|key, _| key.field == field);
    }

    pub(crate) fn touch_visible(&mut self, keys: &[ImageKey]) {
        for key in keys {
            self.cache.get(key);
        }
    }

    pub(crate) fn protect_images(&mut self, mut wanted: Vec<ImageKey>) {
        let previous = wanted
            .iter()
            .filter(|key| {
                !self
                    .cache
                    .entries
                    .iter()
                    .any(|(old, product)| old == *key && product.is_ok())
            })
            .filter_map(|key| {
                self.previous_product(key)
                    .or_else(|| self.source_product(key))
                    .map(|(old, _)| old.clone())
            })
            .collect::<Vec<_>>();
        wanted.extend(previous);
        self.cache.protect(wanted);
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn defer_refresh(&mut self, key: &ImageKey, now: std::time::Instant) -> bool {
        let Some(repaint) = self.repaint.get() else {
            return false;
        };
        if self
            .refresh
            .as_ref()
            .is_none_or(|(waiting, _)| waiting != key)
        {
            self.refresh = Some((key.clone(), now + REFRESH_QUIET_PERIOD));
        }
        let remaining = self
            .refresh
            .as_ref()
            .unwrap()
            .1
            .saturating_duration_since(now);
        if remaining.is_zero() {
            return false;
        }
        repaint.request_repaint_after(remaining);
        true
    }

    pub(crate) fn stopped(&mut self, ticket: u64, key: ImageKey) {
        self.services = None;
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending == ticket)
        {
            self.pending = None;
            self.cache
                .insert(key, Err("Thumbnail worker stopped".into()));
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SourceKey {
    pub(crate) id: SourceId,
    pub(crate) file: FileKey,
    pub(crate) source: SourceDevelopment,
    pub(crate) development: crate::development_metadata::Development,
}

impl SourceKey {
    pub(crate) fn store_key_at(&self, path: &std::path::Path) -> Option<NativeKey> {
        NativeKey::from_revision(path, self.file.clone(), self.development.resolved().ok()?)
    }

    pub(crate) fn store_key(&self) -> Option<NativeKey> {
        self.store_key_at(&self.file.path)
    }

    pub(crate) fn same_pixels(&self, other: &Self) -> bool {
        self.id == other.id
            && self.file == other.file
            && self.source.decode.backend_version == other.source.decode.backend_version
            && self
                .development
                .resolved()
                .ok()
                .zip(other.development.resolved().ok())
                .is_some_and(|(a, b)| a.same_processing(b))
    }

    #[cfg(test)]
    pub(crate) fn metadata_record(&self, path: &std::path::Path) -> Source {
        Source {
            capture_time: None,
            development_mode: self.development.mode(),
            id: self.id,
            path: path.to_path_buf(),
            development: self.development.resolved().unwrap_or(&self.source).clone(),
            adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
                Default::default(),
            )),
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        }
    }

    pub(crate) fn with_development(
        mut self,
        development: crate::development_metadata::Development,
    ) -> Self {
        if let Ok(resolved) = development.resolved() {
            self.source = resolved.for_runtime();
        }
        self.development = development;
        self
    }

    pub(crate) fn new(source: &Source, file: FileKey) -> Self {
        let input = if source.development_mode == crate::project::DevelopmentMode::Unity {
            Default::default()
        } else {
            InputSettings::Digital
        };
        let development = crate::development_metadata::Development::captured(
            source,
            &file.path,
            input,
            &Default::default(),
        );
        Self {
            id: source.id,
            file,
            source: source.development.for_runtime(),
            development,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FieldKey(pub(crate) Arc<crate::flat_field::FlatField>);

impl PartialEq for FieldKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageKey {
    pub(crate) source: SourceKey,
    pub(crate) target: OutputTarget,
    pub(crate) field: Option<FieldKey>,
    pub(crate) recipe: Result<Option<Arc<crate::image_recipe::ImageRecipe>>, String>,
}

pub(crate) struct Product {
    pub(crate) image: Arc<GpuImage>,
    pub(crate) positive: Option<Arc<crate::viewer::view::RenderedView>>,
}

impl std::ops::Deref for Product {
    type Target = GpuImage;

    fn deref(&self) -> &Self::Target {
        &self.image
    }
}

impl Product {
    pub(crate) fn camera(image: Arc<GpuImage>) -> Arc<Self> {
        Arc::new(Self {
            image,
            positive: None,
        })
    }

    pub(crate) fn overhead(positive: bool) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of::<GpuImage>()
            + 1024
            + if positive {
                std::mem::size_of::<crate::viewer::view::ViewImage>()
                    + std::mem::size_of::<crate::viewer::view::RenderedView>()
                    + 2 * std::mem::size_of::<crate::photographic::graph::request::PhotographRequest>(
                    )
            } else {
                0
            }
    }

    pub(crate) fn look_resources(&self) -> Vec<Arc<crate::photographic::stages::look::Lut>> {
        self.positive
            .as_ref()
            .map_or_else(|| self.image.look_resources(), |view| view.look_resources())
    }

    pub(crate) fn byte_len(&self) -> usize {
        let (source, working) = self.image.processing_byte_len();
        let image =
            u64::from(self.image.linear.width()) * u64::from(self.image.linear.height()) * 16;
        let positive = self
            .positive
            .as_ref()
            .map_or(0, |view| view.photographic_bytes());
        (source + working + image + positive) as usize + Self::overhead(self.positive.is_some())
    }
}

impl ImageKey {
    pub(crate) fn luts(&self) -> crate::looks::LutChain {
        self.recipe
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map_or_else(Default::default, |recipe| recipe.luts.clone())
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        let development = &self.source.source.decode;
        std::mem::size_of::<Self>()
            + self.source.file.path.capacity()
            + self.source.source.camera_model.capacity()
            + development.version().len()
            + self
                .field
                .as_ref()
                .map_or(0, |field| field.0.retained_bytes())
            + match &self.recipe {
                Ok(Some(recipe)) => recipe.retained_bytes(),
                Err(reason) => reason.capacity(),
                Ok(None) => 0,
            }
    }

    pub(crate) fn same_presentation(&self, other: &Self) -> bool {
        self.source.same_pixels(&other.source)
            && self.field == other.field
            && self.target == other.target
            && matches!((&self.recipe, &other.recipe), (Ok(Some(a)), Ok(Some(b)))
                if a.composition == b.composition)
    }

    pub(crate) fn estimated_product_bytes(&self) -> Option<usize> {
        let positive = matches!(&self.recipe, Ok(Some(_)));
        let mut bytes =
            256_u64 * 256 * if positive { 64 } else { 48 } + Product::overhead(positive) as u64;
        if let Ok(Some(recipe)) = &self.recipe {
            if recipe.grain.enabled {
                bytes = bytes.checked_add(256 * 256 * 16)?;
            }
            for look in recipe.luts.assets() {
                bytes = bytes
                    .checked_add(256 * 256 * 16)?
                    .checked_add(look.gpu_bytes())?;
            }
            bytes = bytes.checked_add(
                2 * (crate::photographic::stages::grading::Grading::byte_len_for(&recipe.grade)
                    + recipe.grade.retained_bytes() as u64),
            )?;
        }
        usize::try_from(bytes)
            .ok()?
            .checked_add(self.retained_bytes())
    }
}

pub(crate) use crate::scan::CameraPixels as Proxy;

impl Proxy {
    pub(crate) async fn image(
        &self,
        evaluation: crate::evaluation::Context,
        looks: Arc<crate::photographic::stages::look::Looks>,
        transform: &crate::photographic::stages::display_rendering::Target,
        area: [u32; 4],
    ) -> Result<Arc<GpuImage>, Error> {
        let image = GpuImage::new_continuous_shared(
            evaluation,
            looks,
            transform,
            [self.width, self.height],
            &self.pixels,
            self.matrix,
            Some(crate::acquisition::camera::SourceGeometry::new(
                area,
                [self.width, self.height],
            )?),
        )
        .await?;
        Ok(Arc::new(image))
    }
}

pub(crate) type CachedProduct = Result<Arc<Product>, String>;

pub(crate) fn shared_look_bytes(
    entries: &std::collections::VecDeque<(ImageKey, CachedProduct)>,
) -> usize {
    let mut cpu = std::collections::HashSet::new();
    let mut gpu = std::collections::HashSet::new();
    let mut bytes = 0;
    for (key, product) in entries {
        for asset in key.luts().assets() {
            if cpu.insert(asset.allocation()) {
                bytes += asset.numeric_bytes();
            }
        }
        if let Ok(product) = product {
            for lut in product.look_resources() {
                if gpu.insert(Arc::as_ptr(&lut)) {
                    bytes += lut.byte_len() as usize;
                }
            }
        }
    }
    bytes
}

/// Document-owned continuations include CPU/disk work whose GPU ticket has retired.
#[derive(Default)]
pub(crate) struct ThumbnailWork {
    pub(crate) tasks: Vec<Pending>,
}

#[derive(Clone)]
pub(crate) struct Ticket {
    pub(crate) ticket: u64,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

pub(crate) struct Dispatch {
    pub(crate) id: Ticket,
    pub(crate) key: ImageKey,
    retire: crate::cancellation::CancelOnDrop,
}

pub(crate) struct Pending {
    id: Ticket,
    _task: async_executor::Task<()>,
    _retire: crate::cancellation::CancelOnDrop,
}

impl ThumbnailWork {
    #[cfg(test)]
    pub(crate) fn pending(&self) -> bool {
        !self.tasks.is_empty()
    }
}
