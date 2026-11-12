use super::ui::ContactSheetLayout;
use super::*;
use crate::gpu::parameters::ProcessingParameters;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::project::SourceDevelopment;
pub(super) use crate::source::FileKey;
#[cfg(test)]
use crate::source::FileRevision;
use crate::source::SourceKey as NativeKey;
mod disk;
mod lru;
use lru::Cache;
mod embedded;
pub(super) use embedded::Pending as EmbeddedWork;
#[cfg(test)]
mod worker_tests;
#[cfg(test)]
mod workflow_tests;
const SOURCE_SIZE: u32 = Proxy::MAX_EDGE;

#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    id: SourceId,
    file: FileKey,
    source: SourceDevelopment,
    development: crate::development_metadata::Development,
}

impl SourceKey {
    fn store_key_at(&self, path: &std::path::Path) -> Option<NativeKey> {
        NativeKey::from_revision(path, self.file.clone(), self.development.resolved().ok()?)
    }

    fn store_key(&self) -> Option<NativeKey> {
        self.store_key_at(&self.file.path)
    }

    fn same_pixels(&self, other: &Self) -> bool {
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
    fn metadata_record(&self, path: &std::path::Path) -> Source {
        Source {
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

    fn for_document(source: &Source, file: FileKey, document: &Document) -> Self {
        Self::new(source, file).with_development(
            document
                .source_input(source.id, &Default::default())
                .expect("registered photo input")
                .development()
                .expect("photo development"),
        )
    }

    fn with_development(mut self, development: crate::development_metadata::Development) -> Self {
        if let Ok(resolved) = development.resolved() {
            self.source = resolved.for_runtime();
        }
        self.development = development;
        self
    }

    fn new(source: &Source, file: FileKey) -> Self {
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
    recipe: Result<Option<Arc<crate::image_recipe::ImageRecipe>>, String>,
    projection: Option<Composition>,
}

struct Product {
    image: Arc<GpuImage>,
    positive: Option<(
        Arc<crate::gpu::view::ViewSource>,
        Arc<crate::gpu::view::RenderedView>,
    )>,
}

impl std::ops::Deref for Product {
    type Target = GpuImage;

    fn deref(&self) -> &Self::Target {
        &self.image
    }
}

impl Product {
    fn camera(image: Arc<GpuImage>) -> Arc<Self> {
        Arc::new(Self {
            image,
            positive: None,
        })
    }

    fn overhead(positive: bool) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of::<GpuImage>()
            + 1024
            + if positive {
                std::mem::size_of::<crate::gpu::view::ViewSource>()
                    + std::mem::size_of::<crate::gpu::view::RenderedView>()
                    + 2 * std::mem::size_of::<crate::gpu::photograph::request::PhotographRequest>()
            } else {
                0
            }
    }

    fn lut_resources(&self) -> Vec<Arc<crate::gpu::lut3d::Lut>> {
        self.positive.as_ref().map_or_else(
            || self.image.lut_resources(),
            |(_, view)| view.lut_resources(),
        )
    }

    fn byte_len(&self) -> usize {
        let (source, working) = self.image.processing_byte_len();
        let image =
            u64::from(self.image.linear.width()) * u64::from(self.image.linear.height()) * 16;
        let positive = self
            .positive
            .as_ref()
            .map_or(0, |(_, view)| view.photographic_bytes());
        (source + working + image + positive) as usize + Self::overhead(self.positive.is_some())
    }
}

impl ImageKey {
    fn luts(&self) -> crate::looks::LutChain {
        self.recipe
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map_or_else(Default::default, |recipe| recipe.luts.clone())
    }

    fn retained_bytes(&self) -> usize {
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

    fn same_presentation(&self, other: &Self) -> bool {
        self.source.same_pixels(&other.source)
            && self.field == other.field
            && self.target == other.target
            && self.projection == other.projection
            && matches!((&self.recipe, &other.recipe), (Ok(Some(a)), Ok(Some(b)))
                if a.composition == b.composition)
    }

    fn estimated_product_bytes(&self) -> Option<usize> {
        let positive = matches!(&self.recipe, Ok(Some(_)));
        let mut bytes =
            256_u64 * 256 * if positive { 64 } else { 48 } + Product::overhead(positive) as u64;
        if let Ok(Some(recipe)) = &self.recipe {
            if recipe.grain.enabled {
                bytes = bytes.checked_add(256 * 256 * 16)?;
            }
            for look in recipe.luts.assets() {
                bytes = bytes.checked_add(256 * 256 * 16)?.checked_add(
                    (look.lut().nodes().len() * std::mem::size_of::<[f32; 4]>()) as u64,
                )?;
            }
            bytes = bytes.checked_add(
                2 * (crate::gpu::grading::Grading::byte_len_for(&recipe.grade)
                    + recipe.grade.retained_bytes() as u64),
            )?;
        }
        usize::try_from(bytes)
            .ok()?
            .checked_add(self.retained_bytes())
    }

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

pub(super) use crate::scan::CameraPixels as Proxy;

impl Proxy {
    pub(super) async fn image(
        &self,
        renderer: &Arc<crate::renderer::GpuRenderer>,
        device: &wgpu::Device,
        target: OutputTarget,
        area: [u32; 4],
    ) -> Result<Arc<GpuImage>, Error> {
        let image = GpuImage::new_continuous_shared(
            renderer,
            device,
            [self.width, self.height],
            &self.pixels,
            self.matrix,
            Some(crate::gpu::source::SourceGeometry::new(
                area,
                [self.width, self.height],
            )?),
            target,
        )
        .await?;
        Ok(Arc::new(image))
    }
}

pub(super) struct SelectedPreview {
    request: u64,
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

    pub(super) fn current(
        &self,
        request: u64,
        document: &Document,
        source: Option<SourceId>,
    ) -> bool {
        self.request == request && self.current_source(document, source)
    }
}

struct Job {
    cancelled: crate::cancellation::Cancellation,
    reference: Arc<std::sync::OnceLock<crate::exposure_reference::ReferenceExposure>>,
    ticket: u64,
    generation: Arc<std::sync::atomic::AtomicU64>,
    key: ImageKey,
    physical: crate::source::Input,
    correction: Option<Arc<Correction>>,
    render: bool,
    priority: source_loading::Priority,
    previous: Option<Arc<Product>>,
}

struct Outcome {
    proxy: Option<Arc<Proxy>>,
    ticket: u64,
    key: ImageKey,
    image: Result<Option<Arc<Product>>, String>,
    proper: bool,
}

type CachedProduct = Result<Arc<Product>, String>;

fn shared_look_bytes(entries: &std::collections::VecDeque<(ImageKey, CachedProduct)>) -> usize {
    let mut cpu = std::collections::HashSet::new();
    let mut gpu = std::collections::HashSet::new();
    let mut bytes = 0;
    for (key, product) in entries {
        for asset in key.luts().assets() {
            if cpu.insert(asset.lut().nodes().as_ptr()) {
                bytes += asset.lut().byte_len();
            }
        }
        if let Ok(product) = product {
            for lut in product.lut_resources() {
                if gpu.insert(Arc::as_ptr(&lut)) {
                    bytes += lut.byte_len() as usize;
                }
            }
        }
    }
    bytes
}

/// Filmstrip inputs captured by its caller for the current presentation.
#[derive(Default)]
pub(super) struct Preview {
    pub compositions: Vec<(SourceId, Composition)>,
    pub warnings: Vec<(SourceId, String)>,
    pub sources: Vec<(SourceId, Result<ProcessingParameters, String>)>,
    pub outlines: Vec<(SourceId, [[f64; 2]; 4])>,
}

pub(super) struct Thumbnails {
    cache: Cache<ImageKey, CachedProduct>,
    pub(super) sources: Arc<crate::source::SourceCache>,
    selected_key: Option<NativeKey>,
    visible_keys: Vec<NativeKey>,
    // CPU/disk completion history; an admissible GPU miss still needs rendering.
    attempted_sources: Vec<SourceKey>,
    budgets: memory::CacheBudgets,
    view_reservation: usize,
    pub(super) limits: cache_limits::CacheLimits,
    memory_watch: Option<memory::PressureWatch>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    embedded: embedded::EmbeddedPreviews,
    gpu: Option<(
        wgpu::Device,
        wgpu::Queue,
        Arc<crate::renderer::RendererCache>,
    )>,
    services: Option<Services>,
    backend: Option<Arc<dyn crate::source::SourceProvider>>,
    demand: Demand,
    preview: Preview,
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
        let pressure = crate::sys::memory_pressure();
        #[cfg(test)]
        let pressure = memory::Pressure::Normal;
        let budgets = memory::CacheBudgets::new(limits, pressure);
        Self {
            cache: Cache::new(budgets.images, |key: &ImageKey, value: &CachedProduct| {
                key.retained_bytes()
                    - key
                        .luts()
                        .assets()
                        .map(|asset| asset.lut().byte_len())
                        .sum::<usize>()
                    + match value {
                        Ok(product) => {
                            product.byte_len()
                                - product
                                    .lut_resources()
                                    .iter()
                                    .map(|lut| lut.byte_len() as usize)
                                    .sum::<usize>()
                        }
                        Err(reason) => 1024 + reason.capacity(),
                    }
            })
            .with_shared_weight(shared_look_bytes),
            sources: Arc::new(crate::source::SourceCache::new(budgets.sources as u64)),
            selected_key: None,
            visible_keys: Vec::new(),
            attempted_sources: Vec::new(),
            budgets,
            view_reservation: 0,
            limits,
            memory_watch: None,
            repaint: Arc::default(),
            embedded: Default::default(),
            gpu: None,
            services: None,
            backend: None,
            demand: Demand::None,
            preview: Preview::default(),
            generation: Arc::default(),
            pending: None,
            pending_render: false,
            #[cfg(test)]
            pressure_override: Some(memory::Pressure::Normal),
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
        request: u64,
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
        let Some((device, queue, renderers)) = &self.gpu else {
            return Ok(None);
        };
        let renderer = renderers.get(device, queue)?;
        // Selected and filmstrip outputs have independent mutable processing state.
        let preview = SelectedPreview {
            request,
            key: key.clone(),
            proxy: crate::viewer::GpuProxy::new_continuous(
                renderer,
                device,
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
        if !preview.current(request, document, Some(id)) {
            return Ok(None);
        }
        self.selected_key = Some(native_key.clone());
        self.protect_cpu();
        let _ = self.sources.get_proxy(&native_key);
        let bytes = preview.proxy.byte_len();
        log::debug!(
            request,
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
        self.embedded.request_visible(&[], false);
        self.cache.protect(Vec::new());
        self.visible_keys.clear();
        self.protect_cpu();
        self.update_memory();
        self.sync_image_budget();
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
            gpu: self.gpu.clone(),
            services: self.services.clone(),
            backend: self.backend.clone(),
            repaint: self.repaint.clone(),
            embedded: self.embedded.for_document(),
            ..Self::default()
        };
        next.set_limits(self.limits);
        next.sources.set_budget(next.budgets.sources as u64);
        next
    }

    pub(super) fn acquisition(&self) -> Option<Arc<dyn crate::source::SourceProvider>> {
        self.backend
            .as_ref()
            .map(|backend| self.sources.provider(backend.clone()))
    }

    pub(super) fn default_limits(&self) -> cache_limits::CacheLimits {
        cache_limits::Hardware::current(self.gpu.as_ref().map(|(device, _, _)| device)).defaults()
    }

    // Optional reference retention/building shares the existing image-cache allowance.
    // Selected working pixels and active source leases are outside this optional allowance.
    pub(super) fn reserve_view(&mut self, bytes: usize) -> bool {
        let admitted = bytes <= self.budgets.images;
        self.view_reservation = if admitted { bytes } else { 0 };
        self.sync_image_budget();
        admitted
    }

    pub(super) fn set_limits(&mut self, limits: cache_limits::CacheLimits) {
        self.limits = limits;
        self.update_memory();
        self.sync_image_budget();
    }

    pub(super) fn update_memory(&mut self) {
        #[cfg(not(test))]
        let pressure = crate::sys::memory_pressure();
        #[cfg(test)]
        let pressure = self
            .pressure_override
            .unwrap_or_else(crate::sys::memory_pressure);
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
        self.sync_image_budget();
        self.sources.set_budget(self.budgets.sources as u64);
    }

    fn sync_image_budget(&mut self) {
        let available = self.budgets.images.saturating_sub(self.view_reservation);
        self.embedded.set_budget(available.min(16 << 20));
        self.cache
            .set_budget(available.saturating_sub(self.embedded.byte_len()));
        if let Some((_, _, renderers)) = &self.gpu {
            let available = available
                .saturating_sub(self.embedded.byte_len())
                .saturating_sub(self.cache.measured_bytes());
            renderers.set_retention_budget(available as u64);
        }
    }

    fn can_retain(&self, key: &ImageKey) -> bool {
        let Some(mut extra) = key.estimated_product_bytes() else {
            return false;
        };
        for look in key.luts().assets() {
            if self.cache.entries.iter().any(|(old, _)| {
                old.luts()
                    .assets()
                    .any(|old| old.lut().nodes().as_ptr() == look.lut().nodes().as_ptr())
            }) {
                extra -= look.lut().byte_len();
            }
            if self.cache.entries.iter().any(|(_, product)| {
                product.as_ref().ok().is_some_and(|product| {
                    product
                        .lut_resources()
                        .iter()
                        .any(|old| old.id() == look.lut().id())
                })
            }) {
                // The renderer reuses a live immutable GPU LUT by content identity.
                extra -= look.lut().nodes().len() * std::mem::size_of::<[f32; 4]>();
            }
        }
        self.cache.can_retain(extra)
    }

    fn source_product(&self, key: &ImageKey) -> Option<(&ImageKey, &Arc<Product>)> {
        if !matches!(&key.recipe, Ok(Some(_))) {
            return None;
        }
        self.cache.entries.iter().rev().find_map(|(old, product)| {
            let product = product.as_ref().ok()?;
            (key.source.same_pixels(&old.source) && product.positive.is_some())
                .then_some((old, product))
        })
    }

    fn previous_product(&self, key: &ImageKey) -> Option<(&ImageKey, &Arc<Product>)> {
        self.cache.entries.iter().rev().find_map(|(old, product)| {
            key.same_presentation(old)
                .then(|| product.as_ref().ok().map(|product| (old, product)))
                .flatten()
        })
    }

    fn protect_images(&mut self, mut wanted: Vec<ImageKey>) {
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
        self.sync_image_budget();
    }

    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        developments: source_loading::SourceLoader,
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
    pub(super) fn without_disk(&self, developments: source_loading::SourceLoader) -> Self {
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
            source_loading::SourceLoader::new(),
        )
    }

    fn with_developments(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        disk: Option<disk::DiskCache>,
        developments: source_loading::SourceLoader,
    ) -> Self {
        let mut state = Self {
            gpu: Some((device.clone(), queue.clone(), renderers.clone())),
            ..Default::default()
        };
        state.set_limits(state.default_limits());
        state.embedded = embedded::EmbeddedPreviews::new();
        state.backend = Some(Arc::new(developments));
        state.services = Some(Services::Production(Worker::new(
            device,
            queue,
            renderers,
            disk,
            state.repaint.clone(),
        )));
        state
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn projection_ready(
        &self,
        document: &Document,
        source: SourceId,
        target: OutputTarget,
        projection: Composition,
    ) -> bool {
        self.cache.entries.iter().any(|(key, value)| {
            key.source.id == source
                && key.target == target
                && key.projection == Some(projection)
                && key.current_recipe(document)
                && value.as_ref().is_ok_and(|product| {
                    product.positive.is_some() && matches!(product.look_result(), Some(Ok(())))
                })
        })
    }

    #[cfg(test)]
    pub(super) fn ready_count(&self, document: &Document, target: OutputTarget) -> usize {
        self.cache
            .entries
            .iter()
            .filter(|(key, value)| {
                value.is_ok()
                    && key.target == target
                    && key.projection.is_none()
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

    pub fn cancel(&mut self) {
        self.pending = None;
        self.pending_render = false;
        self.embedded.cancel();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn complete(&mut self, outcome: Outcome, document: &Document) {
        let matching = self
            .pending
            .as_ref()
            .is_some_and(|(ticket, _)| *ticket == outcome.ticket);
        if matching {
            self.pending = None;
            self.pending_render = false;
        }
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
        // CPU/disk completion belongs to the document even when its GPU ticket retired.
        if source_current
            && (outcome.proper || (matching && outcome.image.is_err()))
            && !self.attempted_sources.contains(&outcome.key.source)
        {
            self.attempted_sources.push(outcome.key.source.clone());
        }
        if matching
            && source_current
            && self.generation.load(std::sync::atomic::Ordering::Relaxed) == outcome.ticket
            && outcome.key.current_recipe(document)
            && outcome.key.field.as_ref().map(|f| &f.0)
                == document.project.active_flat_field().map(|f| &f.map)
        {
            match outcome.image {
                Ok(Some(image)) => {
                    // A new version replaces this source's old outputs, not another photo.
                    self.cache.retain(|key, _| key.source != outcome.key.source);
                    self.cache.insert(outcome.key, Ok(image));
                }
                Err(error) => self.cache.insert(outcome.key, Err(error)),
                Ok(None) => {}
            }
            self.sync_image_budget();
        }
    }

    fn stopped(&mut self, ticket: u64, key: ImageKey) {
        self.services = None;
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending == ticket)
        {
            self.pending = None;
            self.pending_render = false;
            self.cache
                .insert(key, Err("Thumbnail worker stopped".into()));
        }
    }
}

#[derive(Default, Debug)]
struct ThumbnailMetrics {
    disk_hit: bool,
    neutral_hit: bool,
    calculation_seconds: f64,
    disk_seconds: f64,
    development_seconds: f64,
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
) -> Result<Option<Arc<Product>>, Error> {
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
        job.key.source.source.active_area,
    ))?;
    metrics.image_seconds = tick.elapsed().as_secs_f64();
    Ok(active().then(|| Product::camera(image)))
}

impl Editor {
    fn thumbnail_key(
        &self,
        source: &Source,
        path: &std::path::Path,
        projected: bool,
    ) -> Option<ImageKey> {
        let recipe = self.thumbnail_recipe(source.id);
        let projection = if projected {
            let composition = self.thumbnail_composition(source);
            recipe
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .and_then(|recipe| (recipe.composition != composition).then_some(composition))
        } else {
            None
        };
        Some(ImageKey {
            source: SourceKey::new(source, self.document.file_key(path))
                .with_development(self.document.development_for(source, self.input())),
            recipe,
            projection,
            target: self.document.target(),
            field: self
                .document
                .correction
                .prepared()
                .map(|c| FieldKey(c.settings.map.clone())),
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

    fn thumbnail_work_allowed(&self) -> bool {
        !self.interaction_blocked()
            && self.document.thumbnails.budgets.pressure != memory::Pressure::Critical
            && !self.picking()
            && !self.cropping()
            && !self.masking()
            && !self.files.pending()
            && self.document.export.form.is_none()
            && !self.dialogs.is_open()
    }

    pub(super) fn hidden_filmstrip(&mut self, ui: &egui::Ui, cx: &UiContext<'_, Self>) {
        self.document.thumbnails.embedded.cancel();
        self.document.thumbnails.refresh_hidden_memory();
        self.document.thumbnails.demand = Demand::Hidden;
        self.advance_thumbnail_demand(cx);
        if self.document.thumbnails.pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    pub(super) fn filmstrip(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.filmstrip_preview(ui, Preview::default(), cx);
    }

    pub(super) fn filmstrip_preview(
        &mut self,
        ui: &mut egui::Ui,
        preview: Preview,
        cx: &UiContext<'_, Self>,
    ) {
        let interactive = self.roll_navigation_available();
        if let Some((index, modifiers)) = self.filmstrip_presentation(ui, preview, interactive, cx)
        {
            self.select_roll_photo(index, modifiers, cx);
        }
    }

    pub(super) fn filmstrip_presentation(
        &mut self,
        ui: &mut egui::Ui,
        preview: Preview,
        interactive: bool,
        cx: &UiContext<'_, Self>,
    ) -> Option<(usize, egui::Modifiers)> {
        self.document.thumbnails.preview = preview;
        if self.document.thumbnails.memory_watch.is_none() {
            self.document
                .thumbnails
                .repaint
                .get_or_init(|| ui.ctx().clone());
            self.document.thumbnails.memory_watch =
                Some(memory::PressureWatch::new(ui.ctx().clone()));
        }
        self.document.thumbnails.update_memory();
        self.filmstrip_with_budgets(ui, self.document.thumbnails.budgets, interactive, cx)
    }

    fn filmstrip_with_budgets(
        &mut self,
        ui: &mut egui::Ui,
        budgets: memory::CacheBudgets,
        interactive: bool,
        cx: &UiContext<'_, Self>,
    ) -> Option<(usize, egui::Modifiers)> {
        if self.document.visit.canvas.ready.is_none()
            && self.document.visit.pending_preview.is_none()
        {
            self.document.thumbnails.release_selected();
        }
        let field = self
            .document
            .correction
            .prepared()
            .map(|c| FieldKey(c.settings.map.clone()));
        self.document
            .thumbnails
            .cache
            .retain(|key, _| key.field == field);
        self.document.thumbnails.apply_budgets(budgets);
        let visible_allowed = self.thumbnail_work_allowed();
        let max_height = (ui.ctx().content_rect().height() * 0.35).clamp(52., 240.);
        let mut protected = Vec::new();
        let mut camera_wanted = Vec::new();
        let mut visible = Vec::new();
        let mut select = None;
        let mut remove = false;
        let selection_enabled = interactive;
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
            let count = self.document.state.project.order.len();
            egui::ScrollArea::horizontal().show_viewport(ui, |ui, viewport| {
                let width = (count as f32 * pitch).max(ui.available_width());
                let (content, _) = ui
                    .allocate_exact_size(egui::vec2(width, layout.size().y), egui::Sense::hover());
                let first = (viewport.min.x / pitch).floor().max(0.) as usize;
                let last = (viewport.max.x / pitch).ceil().max(0.) as usize;
                let last = last.min(count);
                for index in first.min(count)..last {
                    visible.push(index);
                    let id = self.document.state.project.order[index];
                    let source = self
                        .document
                        .state
                        .project
                        .source(id)
                        .expect("filmstrip source");
                    let path = self.document.source_path(id).expect("filmstrip locator");
                    let active = self.document.visit.current_source == Some(id);
                    let selected = active || self.document.roll_selection.contains(id);
                    let response_id = ui.id().with(id.0);
                    let card = layout.begin(
                        ui,
                        content.min + egui::vec2(index as f32 * pitch, 0.),
                        response_id,
                        selected,
                        active,
                        selection_enabled,
                    );
                    let image_rect = card.image_rect;
                    let key = self.thumbnail_key(source, &path, true);
                    if let Some(key) = &key {
                        protected.push(key.clone());
                    }
                    let current = key
                        .as_ref()
                        .and_then(|key| self.document.thumbnails.cache.get(key))
                        .cloned();
                    let pending = current.is_none() && key.is_some();
                    // Keep the previous accepted grade visible until its replacement is ready.
                    // Exact-key failures still take precedence over navigation feedback.
                    let entry = current.or_else(|| {
                        key.as_ref().and_then(|key| {
                            self.document
                                .thumbnails
                                .previous_product(key)
                                .map(|(_, product)| Ok(product.clone()))
                                .or_else(|| {
                                    self.document
                                        .thumbnails
                                        .cache
                                        .entries
                                        .iter()
                                        .rev()
                                        .find_map(|(old, value)| {
                                            (key.same_presentation(old) && value.is_err())
                                                .then(|| value.clone())
                                        })
                                })
                        })
                    });
                    let previous = pending && entry.as_ref().is_some_and(Result::is_ok);
                    let mut available = false;
                    let camera_preview = key.as_ref().is_some_and(|key| {
                        self.document.thumbnails.embedded.get(&key.source).is_some()
                    });
                    let parameters = if entry
                        .as_ref()
                        .and_then(|entry| entry.as_ref().ok())
                        .is_some_and(|product| product.positive.is_none())
                    {
                        self.thumbnail_request(source).map(Some)
                    } else {
                        Ok(None)
                    };
                    let mut error = None;
                    match (entry, parameters) {
                        (Some(Ok(image)), Ok(parameters)) => {
                            available = true;
                            let composition = self.thumbnail_composition(source);
                            let dimensions = composition.size();
                            let aspect = dimensions[0] as f32 / dimensions[1] as f32;
                            let size = egui::vec2(
                                (image_rect.height() * aspect).min(image_rect.width()),
                                (image_rect.width() / aspect).min(image_rect.height()),
                            );
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
                                    crate::geometry::FramePoint([
                                        f64::from(fraction.x) * f64::from(dimensions[0]),
                                        f64::from(fraction.y) * f64::from(dimensions[1]),
                                    ])
                                };
                                let mapping = composition.texture_mapping(
                                    frame_point(draw_min),
                                    frame_point(draw_max),
                                    composition.area(),
                                );
                                let failure = match parameters.as_ref() {
                                    Some(input) => {
                                        input.luts().id().and_then(|id| image.look_failure(id))
                                    }
                                    None => image.look_result().and_then(Result::err),
                                };
                                if let Some(failure) = failure {
                                    error = Some(failure.to_string());
                                }
                                match processing::image_paint(
                                    image.image.clone(),
                                    parameters,
                                    self.document
                                        .correction
                                        .prepared()
                                        .map(|correction| correction.gpu.clone()),
                                    mapping,
                                    ui.ctx(),
                                ) {
                                    Ok(paint) => {
                                        ui.painter().with_clip_rect(draw).add(
                                            egui_wgpu::Callback::new_paint_callback(draw, paint),
                                        );
                                    }
                                    Err(message) => {
                                        error = Some(message.to_string());
                                    }
                                }
                            }
                            if let Some((_, q)) = self
                                .document
                                .thumbnails
                                .preview
                                .outlines
                                .iter()
                                .find(|(source, _)| *source == id)
                            {
                                let a = source.development.active_area;
                                let points = q.map(|p| {
                                    full.min
                                        + egui::vec2(
                                            ((p[0] - f64::from(a[0])) / f64::from(a[2])) as f32
                                                * full.width(),
                                            ((p[1] - f64::from(a[1])) / f64::from(a[3])) as f32
                                                * full.height(),
                                        )
                                });
                                ui.painter()
                                    .with_clip_rect(draw)
                                    .add(egui::Shape::closed_line(
                                        points.to_vec(),
                                        ui.visuals().selection.stroke,
                                    ));
                            }
                        }
                        (Some(Err(message)), _) => {
                            error = Some(message);
                        }
                        (_, Err(message)) => {
                            error = Some(message.to_string());
                        }
                        (None, Ok(_)) => {
                            if let Some(key) = key {
                                if let Some(texture) =
                                    self.document.thumbnails.embedded.get(&key.source)
                                {
                                    available = true;
                                    let dimensions = texture.size_vec2();
                                    let scale = (image_rect.width() / dimensions.x)
                                        .min(image_rect.height() / dimensions.y);
                                    let rect = egui::Rect::from_center_size(
                                        image_rect.center(),
                                        dimensions * scale,
                                    );
                                    let painter = ui
                                        .painter()
                                        .with_clip_rect(image_rect.intersect(ui.clip_rect()));
                                    painter.image(
                                        texture.id(),
                                        rect,
                                        egui::Rect::from_min_max(
                                            egui::Pos2::ZERO,
                                            egui::pos2(1., 1.),
                                        ),
                                        egui::Color32::WHITE,
                                    );
                                    card.camera_badge(ui);
                                } else {
                                    card.loading(ui);
                                }
                                camera_wanted.push((index, key.clone(), path.clone()));
                            }
                        }
                    }
                    if error.is_some() {
                        card.error(ui);
                    }
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    card.caption(ui, &name);
                    use ui::{ContextualStatus, StatusLevel};
                    let mut status = if let Some(error) = &error {
                        Some(ContextualStatus::new(
                            StatusLevel::Error,
                            format!("Thumbnail unavailable: {error}"),
                        ))
                    } else if pending && !camera_preview && (!available || previous) {
                        Some(ContextualStatus::new(
                            StatusLevel::Activity,
                            "Preparing thumbnail; navigation preview may differ.",
                        ))
                    } else {
                        None
                    };
                    if let Some(reason) = self
                        .document
                        .thumbnails
                        .preview
                        .warnings
                        .iter()
                        .find(|(source, _)| *source == id)
                        .map(|(_, reason)| reason.clone())
                        .or_else(|| self.source_tool_warning(id))
                    {
                        let warning = ContextualStatus::new(StatusLevel::Warning, reason);
                        status =
                            Some(status.map_or(warning.clone(), |status| status.combine(warning)));
                    }
                    if let Some(status) = status {
                        card.status(ui, &status);
                    }
                    if card.response.clicked() && selection_enabled {
                        select = Some((index, ui.input(|input| input.modifiers)));
                    }
                    if self.roll_edit_available() && card.response.secondary_clicked() && !selected
                    {
                        select = Some((index, egui::Modifiers::NONE));
                    }
                    if self.roll_edit_available()
                        && card.response.enabled()
                        && card.response.has_focus()
                        && !egui::Popup::is_any_open(ui.ctx())
                    {
                        self.select_active_shortcut(ui.ctx());
                        if ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                                || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                        }) {
                            remove = true;
                        }
                    }
                    card.response.context_menu(|ui| {
                        if ui
                            .add_enabled(
                                self.auto_crop_entry_available() && selected,
                                egui::Button::new("Auto-crop selected photos"),
                            )
                            .clicked()
                        {
                            let photos = self
                                .document
                                .project
                                .order
                                .iter()
                                .copied()
                                .filter(|id| self.document.roll_selection.contains(*id))
                                .collect();
                            self.cancel_control_edit();
                            self.review_crop_proposals(photos, "Entry selection", cx);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.roll_edit_available() && !self.tool_interacting() && selected,
                                egui::Button::new("Remove selected photos…"),
                            )
                            .clicked()
                        {
                            remove = true;
                            ui.close();
                        }
                    });
                    let help = error.unwrap_or_else(|| path.display().to_string());
                    card.response
                        .on_hover_text(&help)
                        .on_disabled_hover_text(help);
                }
            });
        });
        if (strip.response.rect.height() - height).abs() > 0.5 {
            self.preferences.filmstrip_height = strip.response.rect.height().clamp(52., 240.);
        }
        if remove {
            self.request_removal(cx);
        }
        self.document.thumbnails.demand = if select.is_none() && !remove {
            Demand::Visible(
                visible
                    .iter()
                    .map(|index| self.document.state.project.order[*index])
                    .collect(),
            )
        } else {
            Demand::None
        };
        self.document.thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        self.document.thumbnails.protect_cpu();
        self.document.thumbnails.protect_images(protected);
        self.request_embedded_previews(
            &camera_wanted,
            visible_allowed && select.is_none() && !remove,
            ui.ctx(),
            cx,
        );
        if select.is_none() && !remove {
            self.advance_thumbnail_demand(cx);
        }
        if self.document.thumbnails.pending.is_some() || self.document.thumbnails.embedded.pending()
        {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        select
    }
}

#[cfg(test)]
struct Submission {
    pub job: Job,
    pub reply: futures_channel::oneshot::Sender<Outcome>,
}

#[derive(Clone)]
enum Services {
    Production(Arc<Worker>),
    #[cfg(test)]
    Controlled(mpsc::Sender<Submission>),
}

#[derive(Debug)]
struct Stopped;

impl Services {
    pub async fn process(
        self,
        job: Job,
        sources: Option<Arc<dyn crate::source::SourceProvider>>,
    ) -> Result<Outcome, Stopped> {
        match self {
            Self::Production(worker) => worker.process(job, sources.ok_or(Stopped)?).await,
            #[cfg(test)]
            Self::Controlled(commands) => {
                let (reply, result) = futures_channel::oneshot::channel();
                commands
                    .send(Submission { job, reply })
                    .map_err(|_| Stopped)?;
                result.await.map_err(|_| Stopped)
            }
        }
    }
}

/// Document-owned continuations include CPU/disk work whose GPU ticket has retired.
#[derive(Default)]
pub(super) struct ThumbnailWork {
    tasks: Vec<Pending>,
}

struct Pending {
    ticket: u64,
    generation: Arc<std::sync::atomic::AtomicU64>,
    _task: async_executor::Task<()>,
    _retire: crate::cancellation::CancelOnDrop,
}

impl ThumbnailWork {
    #[cfg(test)]
    pub(super) fn pending(&self) -> bool {
        !self.tasks.is_empty()
    }
}

/// Visible intent stays lightweight; source/recipe ownership begins with each captured job.
#[derive(Default)]
enum Demand {
    #[default]
    None,
    Visible(Vec<SourceId>),
    Hidden,
}

impl Editor {
    fn request_thumbnail(
        &mut self,
        key: ImageKey,
        render: bool,
        priority: source_loading::Priority,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.thumbnails.pending.is_some()
            || self.document.thumbnails.cache.get(&key).is_some()
        {
            return;
        }
        let Some(services) = self.document.thumbnails.services.clone() else {
            if self.document.thumbnails.gpu.is_some() {
                self.document
                    .thumbnails
                    .cache
                    .insert(key, Err("Thumbnail worker stopped".into()));
            }
            return;
        };
        let generation = self.document.thumbnails.generation.clone();
        let ticket = generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let previous = render
            .then(|| {
                self.document
                    .thumbnails
                    .source_product(&key)
                    .map(|(_, product)| product.clone())
            })
            .flatten();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        let physical = self
            .document
            .source_input(key.source.id)
            .expect("registered thumbnail input");
        let job = Job {
            generation: generation.clone(),
            cancelled,
            reference: physical.reference_exposure(),
            ticket,
            key: key.clone(),
            physical,
            correction: self.document.correction.prepared().cloned(),
            render,
            priority,
            previous,
        };
        self.document.thumbnails.pending_render = render;
        self.document.thumbnails.pending = Some((ticket, key.clone()));
        let fallback_key = key.clone();

        let sources = self.document.thumbnails.acquisition();
        let identity = generation.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let result = services.process(job, sources).await;
            cx.update(|editor| {
                let Some(index) = editor
                    .document
                    .thumbnail_work
                    .tasks
                    .iter()
                    .position(|pending| {
                        pending.ticket == ticket && Arc::ptr_eq(&pending.generation, &identity)
                    })
                else {
                    return;
                };
                let pending = editor.document.thumbnail_work.tasks.swap_remove(index);
                if Arc::ptr_eq(&identity, &editor.document.thumbnails.generation) {
                    editor
                        .document
                        .state
                        .capture_initialization(&editor.document.initialization);
                    match result {
                        Ok(mut outcome) => {
                            if let Some(proxy) = outcome.proxy.take()
                                && let Some(key) = outcome.key.source.store_key()
                                && let Err(error) = editor.document.cache.publish_proxy(key, proxy)
                            {
                                log::warn!(error = error.to_string().as_str(); "thumbnail_proxy_publication_failed");
                            }
                            let current = editor.document.effective_document();
                            editor.document.thumbnails.complete(outcome, &current);
                        }
                        Err(_) => editor.document.thumbnails.stopped(ticket, key),
                    }
                }
                drop(pending);
                editor.document.thumbnails.update_memory();
                editor.advance_thumbnail_demand(&cx);
            });
        }) else {
            self.document.thumbnails.stopped(ticket, fallback_key);
            return;
        };
        self.document.thumbnail_work.tasks.push(Pending {
            ticket,
            generation,
            _task: task,
            _retire: retire,
        });
    }

    pub(super) fn advance_thumbnail_demand(&mut self, cx: &UiContext<'_, Self>) {
        let (visible, hidden) = match &self.document.thumbnails.demand {
            Demand::None => return,
            Demand::Visible(visible) if self.fullscreen.is_none() => (visible.clone(), false),
            Demand::Visible(_) | Demand::Hidden => (Vec::new(), true),
        };

        let allowed = self.thumbnail_work_allowed();
        let eager =
            allowed && self.document.thumbnails.budgets.pressure == memory::Pressure::Normal;
        let idle = allowed
            && !self.document.visit.canvas.original.refining()
            && if self.document.visit.working_view.revision().is_some() {
                self.document.visit.working_view.idle()
            } else {
                self.document.visit.canvas.original.idle()
            };
        let mut protected = Vec::new();
        for id in &visible {
            if let Ok(source) = self.document.state.project.source(*id)
                && let Ok(path) = self.document.source_path(*id)
                && let Some(key) = self.thumbnail_key(source, &path, true)
            {
                protected.push(key);
            }
        }
        if let Some((_, key)) = &self.document.thumbnails.pending {
            let visible = visible.contains(&key.source.id);
            let expected = self
                .document
                .project
                .source(key.source.id)
                .ok()
                .zip(self.document.source_path(key.source.id).ok())
                .and_then(|(source, path)| self.thumbnail_key(source, &path, visible));
            if expected.as_ref() != Some(key)
                || (!visible
                    && (!eager
                        || (self.document.thumbnails.pending_render && !idle)
                        || protected
                            .iter()
                            .any(|key| !self.document.thumbnails.cache.contains(key))))
            {
                self.document.thumbnails.cancel();
            }
        }
        self.document.thumbnails.visible_keys = protected
            .iter()
            .filter_map(|key| key.source.store_key())
            .collect();
        self.document.thumbnails.protect_cpu();
        self.document.thumbnails.protect_images(protected.clone());
        if self.document.thumbnails.services.is_none() {
            if self.document.thumbnails.gpu.is_some() {
                for key in protected {
                    if !self.document.thumbnails.cache.contains(&key) {
                        self.document
                            .thumbnails
                            .cache
                            .insert(key, Err("Thumbnail worker stopped".into()));
                    }
                }
            }
            return;
        }
        if !allowed || self.document.thumbnails.pending.is_some() {
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
        if !hidden {
            candidates.sort_by_key(|index| {
                (
                    !visible_indices.contains(index),
                    visible_indices
                        .iter()
                        .map(|visible| index.abs_diff(*visible))
                        .min()
                        .unwrap_or(0),
                    index.abs_diff(self.document.selected),
                )
            });
        }
        for index in candidates {
            let id = self.document.state.project.order[index];
            let source = self
                .document
                .project
                .source(id)
                .expect("thumbnail demand source");
            let path = self
                .document
                .source_path(id)
                .expect("thumbnail demand locator");
            let visible = visible.contains(&id);
            let Some(key) = self.thumbnail_key(source, &path, visible) else {
                continue;
            };
            if self.document.thumbnails.cache.contains(&key) {
                continue;
            }
            let render = visible || (!hidden && idle && self.document.thumbnails.can_retain(&key));
            if !render
                && self
                    .document
                    .thumbnails
                    .attempted_sources
                    .contains(&key.source)
            {
                continue;
            }
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
            break;
        }
    }
}

enum Input {
    Proxy(Arc<Proxy>),
    Missing,
    Positive,
    Retired,
}

struct Work {
    job: Job,
    input: Result<Input, Error>,
    started: std::time::Instant,
    cache_hit: bool,
    metrics: ThumbnailMetrics,
    prepared: Option<crate::scan::CameraProxy>,
}

struct Worker {
    ready: std::sync::Mutex<ReadyWork>,
    #[cfg(test)]
    observe: Option<Arc<dyn Fn(SourceId, bool) + Send + Sync>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderers: Arc<crate::renderer::RendererCache>,
    disk: Option<disk::DiskCache>,
    repaint: Arc<std::sync::OnceLock<egui::Context>>,
    kernels: std::sync::OnceLock<Result<Arc<crate::gpu::view::ViewKernels>, Error>>,
}

impl Worker {
    fn active(&self, job: &Job) -> bool {
        job.generation.load(std::sync::atomic::Ordering::Relaxed) == job.ticket
            && !job.cancelled.is_cancelled()
    }

    fn begin(&self, job: Job, cached: Option<Arc<Proxy>>) -> Work {
        let started = std::time::Instant::now();
        let cache_hit = cached.is_some();
        let mut metrics = ThumbnailMetrics::default();
        let input = (|| {
            if job.cancelled.is_cancelled() {
                return Ok(Input::Retired);
            }
            let positive = job.render && matches!(&job.key.recipe, Ok(Some(_)));
            if job.render {
                if let Err(reason) = &job.key.recipe {
                    return Err(Error::Gpu(reason.clone()));
                }
                if positive
                    && job
                        .previous
                        .as_ref()
                        .is_some_and(|product| product.positive.is_some())
                {
                    return Ok(Input::Positive);
                }
            }
            let cached = cached.or_else(|| {
                let disk = self.disk.as_ref()?;
                let tick = std::time::Instant::now();
                let result = disk.load(&job.key.source);
                metrics.disk_seconds = tick.elapsed().as_secs_f64();
                match result {
                    Ok(proxy) => {
                        metrics.disk_hit = proxy.is_some();
                        proxy
                    }
                    Err(error) => {
                        log::debug!(
                            source_id = job.key.source.id.0,
                            error = error.to_string().as_str();
                            "thumbnail_disk_cache_miss"
                        );
                        None
                    }
                }
            });
            if let Some(proxy) = cached {
                return Ok(Input::Proxy(proxy));
            }
            if job.cancelled.is_cancelled() {
                return Ok(Input::Retired);
            }
            Ok(Input::Missing)
        })();
        Work {
            job,
            input,
            started,
            cache_hit,
            metrics,
            prepared: None,
        }
    }

    fn positive(
        &self,
        job: &Job,
        recipe: &Arc<crate::image_recipe::ImageRecipe>,
        proxy: Option<&crate::scan::CameraProxy>,
        metrics: &mut ThumbnailMetrics,
    ) -> Result<Option<Arc<Product>>, Error> {
        use crate::gpu::view::RenderedView;
        use crate::gpu::view::ViewKernels;
        use crate::gpu::view::ViewSource;
        use crate::gpu::view::request::SourceRequest;
        if !self.active(job) {
            return Ok(None);
        }
        let mut recipe = recipe.clone();
        Arc::make_mut(&mut recipe).neutral.source_development =
            job.physical.development().expect("photo development");
        let tick = std::time::Instant::now();
        let renderer = self.renderers.get(&self.device, &self.queue)?;
        metrics.renderer_seconds = tick.elapsed().as_secs_f64();
        let submissions = self.renderers.submissions();
        let previous = job
            .previous
            .as_ref()
            .and_then(|product| product.positive.as_ref());
        let (source, produced) = if let Some((source, _)) = previous {
            (source.clone(), None)
        } else {
            let proxy =
                proxy.ok_or_else(|| Error::Gpu("thumbnail requires a camera proxy".into()))?;
            let tick = std::time::Instant::now();
            let mut frame = crate::frame_raster::FrameRaster::full(Composition::new(
                job.key.source.source.active_area.into(),
                Default::default(),
                Default::default(),
            )?)?;
            frame.output = proxy.dimensions();
            let spec = SourceRequest::for_recipe(frame, &recipe)?;
            let capture =
                crate::gpu::photograph::request::PhotographRequest::Recipe(recipe.clone());
            let kernels = self
                .kernels
                .get_or_init(|| pollster::block_on(ViewKernels::new(&self.device)))
                .clone()?;
            let (source, produced) = ViewSource::camera_preview(
                &self.device,
                &self.queue,
                submissions,
                renderer,
                kernels,
                proxy,
                job.correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
                spec,
                &capture,
            )?;
            metrics.calculation_seconds = tick.elapsed().as_secs_f64();
            (source, Some(produced))
        };
        let tick = std::time::Instant::now();
        let capture = crate::gpu::photograph::request::PhotographRequest::Recipe(recipe.clone());
        let (source, view) = if let Some(produced) = produced {
            let view = RenderedView::from_processed(
                &self.device,
                source.clone(),
                capture,
                job.key.target,
                produced,
                previous.map(|(_, view)| view),
            )?;
            (source, view)
        } else {
            RenderedView::render_from_retained(
                &self.device,
                &self.queue,
                submissions,
                source,
                capture,
                job.correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
                job.key.target,
                previous.map(|(_, view)| view),
            )?
        };
        metrics.neutral_hit = previous.is_some_and(|(prior, _)| Arc::ptr_eq(prior, &source));
        view.ensure_submitted(
            &self.device,
            &self.queue,
            submissions,
            self.repaint.get().unwrap_or(&egui::Context::default()),
        );
        if !recipe.luts.is_empty() {
            // Cards retain a full-source raster; admit only their declared composed footprint.
            let composition = job.key.projection.unwrap_or(recipe.composition);
            let mapping = composition.texture_mapping(
                crate::geometry::FramePoint([0.; 2]),
                crate::geometry::FramePoint(composition.size().map(f64::from)),
                source.spec.frame().composition.area(),
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let check = source.renderer.check_look_projection(
                &self.device,
                &mut encoder,
                &view.image.linear,
                mapping,
            )?;
            view.image.set_look_check(Some(check));
            submissions.submit(&self.queue, [encoder.finish()]);
            let repaint = self.repaint.clone();
            view.image.begin_look_check_tracked(submissions, move || {
                if let Some(context) = repaint.get() {
                    context.request_repaint();
                }
            });
        }
        while view.in_flight() {
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|error| Error::Gpu(error.to_string()))?;
            if !self.active(job) {
                return Ok(None);
            }
        }
        view.image
            .look_result()
            .expect("completed thumbnail look check")?;
        metrics.image_seconds = tick.elapsed().as_secs_f64();
        Ok(self.active(job).then(|| {
            Arc::new(Product {
                image: view.image.clone(),
                positive: Some((source, view)),
            })
        }))
    }

    fn finish(&self, mut work: Work) -> Completion {
        let mut proper = false;
        let mut produced = None;
        let mut captured = None;
        let job = &work.job;
        let image = (|| {
            let proxy = match work.input? {
                Input::Positive => {
                    proper = true;
                    let recipe = job
                        .key
                        .recipe
                        .as_ref()
                        .map_err(|error| Error::Gpu(error.clone()))?
                        .as_ref()
                        .ok_or(Error::Gpu("positive thumbnail requires a recipe".into()))?;
                    captured = Some(crate::photo_preparation::ready_image(
                        recipe,
                        &job.reference,
                    ));
                    return self.positive(job, captured.as_ref().unwrap(), None, &mut work.metrics);
                }
                Input::Retired => return Ok(None),
                Input::Proxy(proxy) => proxy,
                Input::Missing => unreachable!("source acquisition precedes proxy work"),
            };

            // Proper CPU/disk completion is document demand, not the disposable GPU ticket.
            if job.cancelled.is_cancelled() {
                return Ok(None);
            }
            produced = Some(proxy.clone());
            if let Some(disk) = &self.disk
                && let Err(error) = disk.store_if_missing(&job.key.source, &proxy)
            {
                log::warn!(
                    source_id = job.key.source.id.0,
                    error = error.to_string().as_str();
                    "thumbnail_disk_cache_write_failed"
                );
            }
            proper = true;
            if !job.render || !self.active(job) {
                return Ok(None);
            }
            if matches!(&job.key.recipe, Ok(Some(_))) {
                let prepared = work.prepared.as_ref().ok_or(Error::UnusableSample(
                    "positive proxy has not been acquired",
                ))?;
                let recipe = job.key.recipe.as_ref().unwrap().as_ref().unwrap();
                // Acquisition resolved the reference; the recipe's recorded value still wins.
                let (prepared, recipe) = crate::photo_preparation::proxy_image(
                    recipe,
                    &job.reference,
                    &prepared.data,
                    Ok(None),
                );
                captured = Some(recipe);
                return self.positive(
                    job,
                    captured.as_ref().unwrap(),
                    Some(&prepared),
                    &mut work.metrics,
                );
            }
            make_image(
                &self.device,
                &self.queue,
                &self.renderers,
                job,
                &proxy,
                &|| self.active(job),
                &mut work.metrics,
            )
        })()
        .map_err(|error: Error| error.to_string());
        log::debug!(
            source_id = job.key.source.id.0,
            target:? = job.key.target,
            source_cache_hit = work.cache_hit,
            disk_cache_hit = work.metrics.disk_hit,
            proper,
            render = job.render,
            cancelled = !self.active(job),
            failed = image.is_err(),
            seconds = work.started.elapsed().as_secs_f64(),
            phases:? = work.metrics;
            "thumbnail"
        );
        let mut key = job.key.clone();
        if let Some(recipe) = captured {
            key.recipe = Ok(Some(recipe));
        }
        Completion {
            outcome: Outcome {
                proxy: None,
                ticket: job.ticket,
                key,
                image,
                proper,
            },
            proxy: produced,
        }
    }
}

struct Completion {
    outcome: Outcome,
    proxy: Option<Arc<Proxy>>,
}

#[derive(Default)]
struct ReadyWork {
    pending: Vec<(Work, futures_channel::oneshot::Sender<Completion>)>,
    running: bool,
}

impl Worker {
    fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderers: Arc<crate::renderer::RendererCache>,
        disk: Option<disk::DiskCache>,
        repaint: Arc<std::sync::OnceLock<egui::Context>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            device,
            queue,
            renderers,
            disk,
            repaint,
            kernels: Default::default(),
            ready: Default::default(),
            #[cfg(test)]
            observe: None,
        })
    }

    async fn process(
        self: Arc<Self>,
        mut job: Job,
        sources: Arc<dyn crate::source::SourceProvider>,
    ) -> Result<Outcome, Stopped> {
        let cancelled = job.cancelled.clone();
        job.key.source = job
            .key
            .source
            .with_development(job.physical.development().expect("photo development"));
        let cached = job
            .key
            .source
            .store_key_at(job.physical.path())
            .and_then(|key| sources.available_proxy(&key));
        let worker = self.clone();
        let mut work = crate::background::run(move |_| worker.begin(job, cached))
            .await
            .map_err(|_| Stopped)?;
        let positive = work.job.render && matches!(&work.job.key.recipe, Ok(Some(_)));
        if positive && matches!(&work.input, Ok(Input::Missing | Input::Proxy(_))) {
            let cached = match &work.input {
                Ok(Input::Proxy(proxy)) => Some(proxy.clone()),
                _ => None,
            };
            let prepared = futures_lite::future::or(
                async {
                    work.job.cancelled.wait().await;
                    Ok(None)
                },
                async {
                    work.job
                        .physical
                        .positive_proxy(sources.as_ref(), work.job.priority, cached)
                        .await
                        .map(Some)
                },
            )
            .await;
            work.input = match prepared {
                Ok(Some(prepared)) => {
                    let pixels = prepared.data.clone();
                    work.prepared = Some(prepared);
                    Ok(Input::Proxy(pixels))
                }
                Ok(None) => Ok(Input::Retired),
                Err(error) => Err(error),
            };
        } else if matches!(&work.input, Ok(Input::Missing)) {
            work.input = futures_lite::future::or(
                async {
                    work.job.cancelled.wait().await;
                    Ok(Input::Retired)
                },
                async {
                    work.job
                        .physical
                        .proxy(sources.as_ref(), work.job.priority)
                        .await
                        .map(Input::Proxy)
                },
            )
            .await;
        }
        work.metrics.development_seconds =
            work.started.elapsed().as_secs_f64() - work.metrics.disk_seconds;
        if let Ok(record) = work.job.physical.record() {
            work.job.key.source.source = record.development.for_runtime();
        }
        let (reply, result) = futures_channel::oneshot::channel();
        let start = {
            let mut ready = self.ready.lock().unwrap();
            ready.pending.push((work, reply));
            let start = !ready.running;
            ready.running = true;
            start
        };
        if start {
            // One pool job drains ready CPU/GPU work; native/preview waits stay in futures.
            blocking::unblock(move || self.drain()).detach();
        }
        let completion = result.await.map_err(|_| Stopped)?;
        let mut outcome = completion.outcome;
        if !cancelled.is_cancelled() {
            outcome.proxy = completion.proxy;
        }
        Ok(outcome)
    }

    fn drain(&self) {
        loop {
            let next = {
                let mut ready = self.ready.lock().unwrap();
                let index = ready
                    .pending
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (work, _))| work.job.priority)
                    .map(|(index, _)| index);
                match index {
                    Some(index) => Some(ready.pending.remove(index)),
                    None => {
                        ready.running = false;
                        None
                    }
                }
            };
            let Some((work, reply)) = next else {
                return;
            };
            if let Ok(outcome) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #[cfg(test)]
                let id = work.job.key.source.id;
                #[cfg(test)]
                if let Some(observe) = &self.observe {
                    observe(id, true);
                }
                let outcome = self.finish(work);
                #[cfg(test)]
                if let Some(observe) = &self.observe {
                    observe(id, false);
                }
                outcome
            })) {
                let _ = reply.send(outcome);
            }
        }
    }
}

#[cfg(test)]
mod clipping_tests;
#[cfg(all(test, target_os = "macos"))]
use crate::cache_observation as observation;
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
