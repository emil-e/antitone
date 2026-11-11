//! One scoped lookup for live production, active images and optionally retained image storage.
use super::*;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared, WeakShared},
};
use std::{
    any::Any,
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Mutex, Weak},
    task::{Context as TaskContext, Poll},
};

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Rgba16Uint,
    Rgba32Float,
}
impl ImageFormat {
    pub fn pixel_bytes(self) -> u64 {
        match self {
            Self::Rgba16Uint => 8,
            Self::Rgba32Float => 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageBacking {
    Buffer,
    Texture,
    Host,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageLayout {
    pub domain: Domain,
    pub region: CellRegion,
    pub format: ImageFormat,
    pub backing: ImageBacking,
}
impl ImageLayout {
    pub fn bytes(self) -> u64 {
        u64::from(self.region.width) * u64::from(self.region.height) * self.format.pixel_bytes()
    }
    pub fn supplies(self, requested: Self) -> bool {
        self.domain == requested.domain
            && self.format == requested.format
            && self.backing == requested.backing
            && self.region.contains_region(requested.region)
    }
}

#[derive(Debug)]
pub(super) enum ImageStorage {
    Buffer(wgpu::Buffer),
    Texture(wgpu::Texture),
    Host(Arc<[[f32; 4]]>),
}
impl PartialEq for ImageStorage {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Buffer(a), Self::Buffer(b)) => a == b,
            (Self::Texture(a), Self::Texture(b)) => a == b,
            (Self::Host(a), Self::Host(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
impl Eq for ImageStorage {}
pub struct ImageValue {
    pub(super) storage: ImageStorage,
    // The charge follows physical image owners, including recorded consumers, not UI retention.
    admission: std::sync::OnceLock<Arc<cache::Lease>>,
}
impl std::fmt::Debug for ImageValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageValue")
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}
impl PartialEq for ImageValue {
    fn eq(&self, other: &Self) -> bool {
        self.storage == other.storage
    }
}
impl Eq for ImageValue {}

impl ImageValue {
    fn stored(storage: ImageStorage) -> Arc<Self> {
        Arc::new(Self {
            storage,
            admission: Default::default(),
        })
    }
    pub fn texture(texture: wgpu::Texture) -> Arc<Self> {
        Self::stored(ImageStorage::Texture(texture))
    }
    pub(super) fn admit(&self, lease: Arc<cache::Lease>) {
        let _ = self.admission.set(lease);
    }
    pub(super) fn admitted(&self) -> bool {
        self.admission.get().is_some()
    }

    pub fn host(pixels: Vec<[f32; 4]>) -> Arc<Self> {
        Self::stored(ImageStorage::Host(pixels.into()))
    }
    pub fn as_host(&self) -> Option<&[[f32; 4]]> {
        match &self.storage {
            ImageStorage::Host(pixels) => Some(pixels),
            _ => None,
        }
    }
    pub fn new(pixels: wgpu::Buffer) -> Arc<Self> {
        Self::stored(ImageStorage::Buffer(pixels))
    }
    pub fn as_buffer(&self) -> Option<&wgpu::Buffer> {
        match &self.storage {
            ImageStorage::Buffer(pixels) => Some(pixels),
            _ => None,
        }
    }
    pub fn as_texture(&self) -> Option<&wgpu::Texture> {
        match &self.storage {
            ImageStorage::Texture(pixels) => Some(pixels),
            _ => None,
        }
    }
    pub(super) fn binding(self: &Arc<Self>) -> super::Shared {
        super::Shared::Image {
            image: self.clone(),
            view: self
                .as_texture()
                .map(|texture| texture.create_view(&Default::default())),
        }
    }

    fn bytes(&self) -> Option<u64> {
        match &self.storage {
            ImageStorage::Host(pixels) => Some(pixels.len() as u64 * 16),
            ImageStorage::Buffer(pixels) => pixels
                .usage()
                .contains(wgpu::BufferUsages::STORAGE)
                .then(|| pixels.size()),
            ImageStorage::Texture(pixels) => {
                if pixels.dimension() != wgpu::TextureDimension::D2
                    || pixels.mip_level_count() != 1
                    || pixels.sample_count() != 1
                    || !pixels
                        .usage()
                        .contains(wgpu::TextureUsages::TEXTURE_BINDING)
                {
                    return None;
                }
                let pixel_bytes = match pixels.format() {
                    wgpu::TextureFormat::Rgba16Uint => 8,
                    wgpu::TextureFormat::Rgba32Float => 16,
                    _ => return None,
                };
                let size = pixels.size();
                Some(
                    u64::from(size.width)
                        * u64::from(size.height)
                        * u64::from(size.depth_or_array_layers)
                        * pixel_bytes,
                )
            }
        }
    }
}
type ImageResult = Result<Arc<ImageValue>, Arc<Error>>;
type SharedResult = Shared<BoxFuture<'static, ImageResult>>;

pub struct Image<D> {
    pub identity: RetainedKey,
    pub descriptor: Arc<D>,
    pub image: Arc<ImageValue>,
}
impl<D> Clone for Image<D> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            descriptor: self.descriptor.clone(),
            image: self.image.clone(),
        }
    }
}

/// A buffer view that keeps its captured image value alive when one is available.
#[derive(Clone)]
pub(crate) enum BufferImage {
    Produced(Image<ImageLayout>),
    Submitted(wgpu::Buffer),
}
impl std::fmt::Debug for BufferImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
#[cfg(feature = "editor")]
impl BufferImage {
    pub(crate) fn recorded(
        definition: &super::Value,
        region: CellRegion,
        pixels: wgpu::Buffer,
    ) -> Result<Self, Error> {
        let output = definition.output(region)?;
        let Some(identity) = output.identity() else {
            return Ok(Self::Submitted(pixels));
        };
        let layout = ImageLayout {
            domain: definition.domain(),
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let image = ImageValue::new(pixels);
        super::materialized::validate(layout.domain, layout, &image)?;
        Ok(Self::Produced(Image {
            identity: identity.clone(),
            descriptor: Arc::new(layout),
            image,
        }))
    }

    pub(crate) fn submitted(&self, scope: &Materializations) {
        if let Self::Produced(image) = self {
            scope.publish(image, false);
        }
    }
}

impl From<wgpu::Buffer> for BufferImage {
    fn from(value: wgpu::Buffer) -> Self {
        Self::Submitted(value)
    }
}
impl TryFrom<Image<ImageLayout>> for BufferImage {
    type Error = Error;
    fn try_from(image: Image<ImageLayout>) -> Result<Self, Error> {
        if image.as_buffer().is_none() {
            return Err(Error::InvalidParameter(
                "buffer image requires buffer storage",
            ));
        }
        Ok(Self::Produced(image))
    }
}
impl std::ops::Deref for BufferImage {
    type Target = wgpu::Buffer;
    fn deref(&self) -> &wgpu::Buffer {
        match self {
            Self::Produced(image) => image.as_buffer().expect("validated buffer image"),
            Self::Submitted(buffer) => buffer,
        }
    }
}
impl PartialEq for BufferImage {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<D> std::ops::Deref for Image<D> {
    type Target = ImageValue;
    fn deref(&self) -> &ImageValue {
        &self.image
    }
}
pub(super) struct InlineProducer {
    scope: Weak<State>,
    token: Arc<()>,
    result: WeakShared<BoxFuture<'static, ImageResult>>,
    send: Option<futures_channel::oneshot::Sender<ImageResult>>,
    stopped: std::sync::atomic::AtomicBool,
}
impl InlineProducer {
    pub(super) fn cancel_when_unshared(&self) -> bool {
        if self.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return true;
        }
        let scope = self.scope.upgrade();
        let mut entries = scope.as_ref().map(|scope| scope.entries.lock().unwrap());
        // The caller owns one future; this weak upgrade adds one temporary owner.
        // Retire under the store lock so new requests cannot join a stopping producer.
        let owners = self
            .result
            .upgrade()
            .and_then(|result| result.strong_count())
            .unwrap_or(0);
        if owners > 2 {
            return false;
        }
        if let Some(entries) = entries.as_mut() {
            entries.retain(|entry| !Arc::ptr_eq(&entry.token, &self.token));
        }
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
        true
    }
    pub(super) fn complete(mut self, result: Result<Arc<ImageValue>, Error>) {
        let _ = self.send.take().unwrap().send(result.map_err(Arc::new));
    }
}
impl Drop for InlineProducer {
    fn drop(&mut self) {
        if let Some(send) = self.send.take() {
            let _ = send.send(Err(Arc::new(Error::Cancelled)));
        }
    }
}

pub struct ImageFuture<D> {
    identity: RetainedKey,
    descriptor: Arc<D>,
    result: SharedResult,
}
impl<D> Clone for ImageFuture<D> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            descriptor: self.descriptor.clone(),
            result: self.result.clone(),
        }
    }
}
impl<D> Future for ImageFuture<D> {
    type Output = Result<Image<D>, Arc<Error>>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.result).poll(cx).map(|result| {
            result.map(|image| Image {
                identity: self.identity.clone(),
                descriptor: self.descriptor.clone(),
                image,
            })
        })
    }
}

#[derive(Clone)]
pub struct Materializations(Arc<State>);
struct State {
    programs: Arc<Programs>,
    retention: cache::Allowance,
    retention_enabled: std::sync::atomic::AtomicBool,
    #[cfg(all(test, feature = "raw"))]
    pressure_override: Mutex<Option<crate::sys::MemoryPressure>>,
    entries: Mutex<VecDeque<Entry>>,
}
#[derive(Clone, Copy)]
pub(super) struct RequestPolicy {
    pub budget: Option<Budget>,
    pub retain: bool,
}

struct Entry {
    token: Arc<()>,
    key: RetainedKey,
    descriptor: Arc<dyn Any + Send + Sync>,
    value: EntryState,
    budget: Option<Budget>,
    retain: bool,
    retained: Option<Arc<ImageValue>>,
    lease: Option<Arc<cache::Lease>>,
    initializer: Option<Weak<cache::RetainedBuffer>>,
}
enum EntryState {
    Recording {
        result: WeakShared<BoxFuture<'static, ImageResult>>,
    },
    Producing {
        result: WeakShared<BoxFuture<'static, ImageResult>>,
        inline: bool,
    },
    Complete(Weak<ImageValue>),
}
impl EntryState {
    fn result(&self) -> Option<SharedResult> {
        match self {
            Self::Recording { .. } => None,
            Self::Producing { result, .. } => result.upgrade(),
            Self::Complete(weak) => Some(
                futures_util::future::ready(Ok(weak.upgrade()?))
                    .boxed()
                    .shared(),
            ),
        }
    }
    fn image(&self) -> Option<Arc<ImageValue>> {
        match self {
            Self::Complete(weak) => weak.upgrade(),
            _ => None,
        }
    }
}

fn sweep(entries: &mut VecDeque<Entry>) {
    entries.retain(|entry| match &entry.value {
        EntryState::Producing { result, .. } | EntryState::Recording { result } => {
            result.upgrade().is_some()
        }
        EntryState::Complete(weak) => weak.strong_count() != 0,
    });
}

impl Materializations {
    pub(super) fn programs(&self) -> &Arc<Programs> {
        &self.0.programs
    }

    pub(super) fn new(programs: Arc<Programs>, retention: cache::Allowance) -> Self {
        Self(Arc::new(State {
            programs,
            retention,
            retention_enabled: true.into(),
            #[cfg(all(test, feature = "raw"))]
            pressure_override: Mutex::new(None),
            entries: Mutex::new(VecDeque::new()),
        }))
    }
    pub(crate) fn belongs_to(&self, context: &Context) -> bool {
        Arc::ptr_eq(&self.0.programs, &context.programs)
    }
    pub fn len(&self) -> usize {
        let mut entries = self.0.entries.lock().unwrap();
        sweep(&mut entries);
        entries.len()
    }
    #[cfg(all(test, feature = "editor"))]
    pub(super) fn pending_consumers<D: Any + Send + Sync>(
        &self,
        key: &RetainedKey,
        matches: impl Fn(&D) -> bool,
    ) -> usize {
        self.0
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| {
                entry.key == *key && entry.descriptor.downcast_ref::<D>().is_some_and(&matches)
            })
            .filter_map(|entry| match &entry.value {
                EntryState::Producing { result: value, .. } => value
                    .upgrade()
                    .and_then(|value| value.strong_count())
                    .map(|count| count.saturating_sub(1)),
                _ => None,
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn clear(&self) {
        self.0.entries.lock().unwrap().clear();
    }

    #[cfg(feature = "editor")]
    pub(crate) fn set_retention_limit(&self, bytes: u64) {
        let mut entries = self.0.entries.lock().unwrap();
        self.0.retention.set_limit(bytes);
        for entry in entries.iter_mut() {
            if self.0.retention.bytes() <= bytes {
                break;
            }
            entry.retained = None;
            entry.lease = None;
        }
        sweep(&mut entries);
    }

    pub(crate) fn set_retention_enabled(&self, enabled: bool) {
        let mut entries = self.0.entries.lock().unwrap();
        self.0
            .retention_enabled
            .store(enabled, std::sync::atomic::Ordering::Release);
        if !enabled {
            for entry in entries.iter_mut() {
                entry.retained = None;
                entry.lease = None;
            }
            sweep(&mut entries);
        }
    }

    #[cfg(all(test, feature = "raw"))]
    pub(crate) fn set_test_pressure(&self, pressure: crate::sys::MemoryPressure) {
        *self.0.pressure_override.lock().unwrap() = Some(pressure);
        self.pressure(pressure);
    }

    #[cfg(feature = "raw")]
    fn pressure(&self, pressure: crate::sys::MemoryPressure) {
        #[cfg(test)]
        let fixed = self.0.pressure_override.lock().unwrap();
        #[cfg(test)]
        let pressure = fixed.unwrap_or(pressure);
        self.set_retention_enabled(pressure == crate::sys::MemoryPressure::Normal);
    }

    #[cfg(feature = "raw")]
    pub(crate) fn watch_pressure(&self) -> crate::sys::MemoryPressureWatch {
        self.pressure(crate::sys::memory_pressure());
        let weak = Arc::downgrade(&self.0);
        crate::sys::MemoryPressureWatch::observe(move |pressure| {
            if let Some(state) = weak.upgrade() {
                Self(state).pressure(pressure);
            }
        })
    }

    fn find_entry<D: Any + Send + Sync>(
        &self,
        entries: &mut VecDeque<Entry>,
        key: &RetainedKey,
        can_supply: impl Fn(&D) -> bool,
        policy: RequestPolicy,
    ) -> Option<ImageFuture<D>> {
        let budget = policy.budget;
        sweep(entries);
        let index = entries.iter().rposition(|entry| {
            entry.key == *key
                && !matches!(entry.value, EntryState::Recording { .. })
                && (matches!(entry.value, EntryState::Complete(_))
                    || budget.is_none()
                    || entry.budget.is_none()
                    || entry.budget == budget)
                && entry
                    .descriptor
                    .downcast_ref::<D>()
                    .is_some_and(&can_supply)
        })?;
        let mut entry = entries.remove(index).unwrap();
        let image = ImageFuture {
            identity: entry.key.clone(),
            descriptor: entry.descriptor.clone().downcast().ok()?,
            result: entry.value.result()?,
        };
        entry.retain |= policy.retain;
        if policy.retain
            && let Some(value) = entry.value.image()
        {
            let bytes = value.bytes()?;
            self.completed(entries, entry, &value, bytes);
        } else {
            entries.push_back(entry);
        }
        Some(image)
    }
    pub fn find<D: Any + Send + Sync>(
        &self,
        key: &RetainedKey,
        can_supply: impl Fn(&D) -> bool,
    ) -> Option<ImageFuture<D>> {
        self.find_entry(
            &mut self.0.entries.lock().unwrap(),
            key,
            can_supply,
            RequestPolicy {
                budget: None,
                retain: false,
            },
        )
    }
    pub(super) fn find_for<D: Any + Send + Sync>(
        &self,
        key: &RetainedKey,
        can_supply: impl Fn(&D) -> bool,
        policy: RequestPolicy,
    ) -> Option<ImageFuture<D>> {
        self.find_entry(&mut self.0.entries.lock().unwrap(), key, can_supply, policy)
    }
    pub(super) fn ready<D: Any + Send + Sync>(&self, key: &RetainedKey) -> Vec<Image<D>> {
        let mut entries = self.0.entries.lock().unwrap();
        sweep(&mut entries);
        let found: Vec<_> = entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if entry.key != *key {
                    return None;
                }
                Some((
                    index,
                    Image {
                        identity: entry.key.clone(),
                        descriptor: entry.descriptor.clone().downcast::<D>().ok()?,
                        image: entry.value.image()?,
                    },
                ))
            })
            .collect();
        let recent: Vec<_> = found
            .iter()
            .rev()
            .map(|(index, _)| entries.remove(*index).unwrap())
            .collect();
        entries.extend(recent.into_iter().rev());
        found.into_iter().rev().map(|(_, image)| image).collect()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn find_ready<D: Any + Send + Sync>(
        &self,
        key: &RetainedKey,
        can_supply: impl Fn(&D) -> bool,
    ) -> Option<Image<D>> {
        self.ready::<D>(key)
            .into_iter()
            .find(|image| can_supply(image.descriptor.as_ref()))
    }
    #[cfg(feature = "editor")]
    pub(super) fn release_optional(&self, key: &RetainedKey) {
        let mut entries = self.0.entries.lock().unwrap();
        for entry in entries.iter_mut().filter(|entry| entry.key == *key) {
            entry.retained = None;
            entry.lease = None;
        }
        sweep(&mut entries);
    }

    fn reserve(&self, entries: &mut VecDeque<Entry>, bytes: u64) -> Option<Arc<cache::Lease>> {
        if !self
            .0
            .retention_enabled
            .load(std::sync::atomic::Ordering::Acquire)
            || bytes == 0
            || bytes > self.0.retention.limit()
        {
            return None;
        }
        loop {
            if let Some(lease) = self.0.retention.acquire(bytes) {
                return Some(Arc::new(lease));
            }
            let victim = entries.iter_mut().find(|entry| entry.retained.is_some())?;
            victim.retained = None;
            victim.lease = None;
            sweep(entries);
        }
    }
    fn completed(
        &self,
        entries: &mut VecDeque<Entry>,
        mut entry: Entry,
        image: &Arc<ImageValue>,
        bytes: u64,
    ) {
        entry.value = EntryState::Complete(Arc::downgrade(image));
        if !entry.retain || image.admitted() {
            entry.lease = None;
        } else if entry.lease.is_none() {
            entry.lease = self.reserve(entries, bytes);
        }
        entry.retained = entry.lease.as_ref().map(|_| image.clone());
        entries.push_back(entry);
    }

    #[cfg(all(test, feature = "editor"))]
    pub(in crate::gpu::evaluation) fn submitted_image(&self, image: &Image<ImageLayout>) {
        self.publish(image, true);
    }

    /// Phase inputs stay discoverable through their round owners without displacing
    /// retained calculation results. An independent retained request keeps its ownership.
    pub(in crate::gpu::evaluation) fn submitted_input(&self, image: &Image<ImageLayout>) {
        self.publish(image, false);
    }

    pub(in crate::gpu::evaluation) fn publish(&self, image: &Image<ImageLayout>, retain: bool) {
        let mut entries = self.0.entries.lock().unwrap();
        sweep(&mut entries);
        let existing = entries.iter().position(|entry| {
            entry.key == image.identity
                && entry.descriptor.downcast_ref::<ImageLayout>() == Some(image.descriptor.as_ref())
                && match &entry.value {
                    EntryState::Producing { .. } => true,
                    EntryState::Recording { .. } => false,
                    EntryState::Complete(value) => value
                        .upgrade()
                        .is_some_and(|value| Arc::ptr_eq(&value, &image.image)),
                }
        });
        let mut entry = existing
            .map(|index| entries.remove(index).unwrap())
            .unwrap_or_else(|| Entry {
                token: Arc::new(()),
                key: image.identity.clone(),
                descriptor: image.descriptor.clone(),
                value: EntryState::Complete(Arc::downgrade(&image.image)),
                budget: None,
                retain: false,
                retained: None,
                lease: None,
                initializer: None,
            });
        entry.retain |= retain;
        self.completed(&mut entries, entry, &image.image, image.descriptor.bytes());
    }

    /// Live requests share production even when optional ownership is disabled or exhausted.
    pub fn obtain<D, F, Fut>(
        &self,
        key: RetainedKey,
        descriptor: Arc<D>,
        bytes: u64,
        produce: F,
    ) -> ImageFuture<D>
    where
        D: Any + Send + Sync + PartialEq,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Arc<ImageValue>, Error>> + Send + 'static,
    {
        self.obtain_for(
            key,
            descriptor,
            bytes,
            RequestPolicy {
                budget: None,
                retain: true,
            },
            produce,
        )
    }

    pub(super) fn obtain_for<D, F, Fut>(
        &self,
        key: RetainedKey,
        descriptor: Arc<D>,
        bytes: u64,
        policy: RequestPolicy,
        produce: F,
    ) -> ImageFuture<D>
    where
        D: Any + Send + Sync + PartialEq,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Arc<ImageValue>, Error>> + Send + 'static,
    {
        let mut entries = self.0.entries.lock().unwrap();
        if let Some(image) =
            self.find_entry::<D>(&mut entries, &key, |d| d == descriptor.as_ref(), policy)
        {
            return image;
        }
        self.create(&mut entries, key, descriptor, bytes, policy, async move {
            produce().await.map_err(Arc::new)
        })
    }

    /// Claim work on the current worker, or join an already running unit.
    pub(super) fn claim_unit(
        &self,
        key: RetainedKey,
        layout: Arc<ImageLayout>,
        budget: Budget,
    ) -> (ImageFuture<ImageLayout>, Option<InlineProducer>) {
        let mut entries = self.0.entries.lock().unwrap();
        // A queued root/batch cannot be awaited by a worker it may depend on. Fulfil its
        // calculation here; when its continuation runs, it can reuse the completed value.
        let mut reserved_retention = None;
        entries.retain(|entry| {
            let queued = entry.key == key
                && entry.descriptor.downcast_ref::<ImageLayout>() == Some(layout.as_ref())
                && (entry.budget.is_none() || entry.budget == Some(budget))
                && matches!(entry.value, EntryState::Producing { inline: false, .. });
            if queued {
                reserved_retention = Some(reserved_retention.unwrap_or(false) || entry.retain);
            }
            !queued
        });
        let policy = RequestPolicy {
            budget: Some(budget),
            retain: reserved_retention.unwrap_or(true),
        };
        if let Some(value) = self.find_entry(
            &mut entries,
            &key,
            |stored: &ImageLayout| stored == layout.as_ref(),
            policy,
        ) {
            return (value, None);
        }
        let (send, receive) = futures_channel::oneshot::channel();
        let bytes = layout.bytes();
        let value = self.create(&mut entries, key, layout, bytes, policy, async move {
            receive
                .await
                .unwrap_or_else(|_| Err(Arc::new(Error::Cancelled)))
        });
        let entry = entries.back_mut().unwrap();
        let EntryState::Producing { inline, .. } = &mut entry.value else {
            unreachable!()
        };
        *inline = true;
        let producer = InlineProducer {
            scope: Arc::downgrade(&self.0),
            token: entry.token.clone(),
            result: value.result.downgrade().unwrap(),
            send: Some(send),
            stopped: false.into(),
        };
        (value, Some(producer))
    }

    fn create<D: Any + Send + Sync>(
        &self,
        entries: &mut VecDeque<Entry>,
        key: RetainedKey,
        descriptor: Arc<D>,
        bytes: u64,
        policy: RequestPolicy,
        produce: impl Future<Output = ImageResult> + Send + 'static,
    ) -> ImageFuture<D> {
        let token = Arc::new(());
        let producing = token.clone();
        let scope = Arc::downgrade(&self.0);
        let result = async move {
            let result = produce.await.and_then(|image| {
                if image.bytes() != Some(bytes) {
                    return Err(Arc::new(Error::InvalidParameter(
                        "materialization differs from admitted storage",
                    )));
                }
                Ok(image)
            });
            if let Some(scope) = scope.upgrade() {
                let scope = Self(scope);
                let mut entries = scope.0.entries.lock().unwrap();
                if let Some(index) = entries
                    .iter()
                    .position(|entry| Arc::ptr_eq(&entry.token, &producing))
                {
                    let entry = entries.remove(index).unwrap();
                    if let Ok(image) = &result {
                        scope.completed(&mut entries, entry, image, bytes);
                    }
                }
            }
            result
        }
        .boxed()
        .shared();
        entries.push_back(Entry {
            token,
            key: key.clone(),
            descriptor: descriptor.clone(),
            value: EntryState::Producing {
                result: result.downgrade().unwrap(),
                inline: false,
            },
            budget: policy.budget,
            retain: policy.retain,
            retained: None,
            lease: None,
            initializer: None,
        });
        ImageFuture {
            identity: key,
            descriptor,
            result,
        }
    }

    /// Reserve related outputs atomically; existing consumers keep their independent production.
    /// The producer receives only newly reserved request indices, including duplicate coalescing.
    pub fn obtain_many<D, F, Fut>(
        &self,
        requests: Vec<(RetainedKey, Arc<D>, u64)>,
        produce: F,
    ) -> Vec<ImageFuture<D>>
    where
        D: Any + Send + Sync + PartialEq,
        F: FnOnce(Vec<usize>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Vec<Arc<ImageValue>>, Error>> + Send + 'static,
    {
        self.obtain_many_for(
            requests
                .into_iter()
                .map(|(key, layout, bytes)| {
                    (
                        key,
                        layout,
                        bytes,
                        RequestPolicy {
                            budget: None,
                            retain: true,
                        },
                    )
                })
                .collect(),
            produce,
        )
    }

    pub(super) fn obtain_many_for<D, F, Fut>(
        &self,
        requests: Vec<(RetainedKey, Arc<D>, u64, RequestPolicy)>,
        produce: F,
    ) -> Vec<ImageFuture<D>>
    where
        D: Any + Send + Sync + PartialEq,
        F: FnOnce(Vec<usize>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Vec<Arc<ImageValue>>, Error>> + Send + 'static,
    {
        type Batch = Shared<BoxFuture<'static, Result<Vec<Arc<ImageValue>>, Arc<Error>>>>;
        let batch = Arc::new(std::sync::OnceLock::<Batch>::new());
        let mut entries = self.0.entries.lock().unwrap();
        let mut missing = Vec::new();
        let mut outputs = Vec::new();
        for (index, (key, descriptor, bytes, policy)) in requests.into_iter().enumerate() {
            let output = if let Some(found) =
                self.find_entry::<D>(&mut entries, &key, |d| d == descriptor.as_ref(), policy)
            {
                found
            } else {
                let slot = missing.len();
                missing.push(index);
                let batch = batch.clone();
                self.create(&mut entries, key, descriptor, bytes, policy, async move {
                    batch
                        .get()
                        .expect("batch registered before publication")
                        .clone()
                        .await?
                        .get(slot)
                        .cloned()
                        .ok_or_else(|| {
                            Arc::new(Error::InvalidParameter("batch omitted a requested image"))
                        })
                })
            };
            outputs.push(output);
        }
        let work = async move { produce(missing).await.map_err(Arc::new) }
            .boxed()
            .shared();
        assert!(batch.set(work).is_ok());
        outputs
    }

    pub(crate) fn generated_image(
        &self,
        device: &wgpu::Device,
        key: &RetainedKey,
        layout: ImageLayout,
        retain: bool,
        required: bool,
    ) -> Option<Arc<cache::RetainedBuffer>> {
        let descriptor = layout;
        let bytes = layout.bytes();
        let mut entries = self.0.entries.lock().unwrap();
        sweep(&mut entries);
        let existing = entries.iter().position(|entry| {
            entry.key == *key && entry.descriptor.downcast_ref::<ImageLayout>() == Some(&descriptor)
        });
        if let Some(index) = existing {
            let mut entry = entries.remove(index).unwrap();
            let resource = entry
                .initializer
                .as_ref()
                .and_then(Weak::upgrade)
                .or_else(|| {
                    let image = entry.value.image()?;
                    Some(Arc::new(cache::RetainedBuffer::reused(
                        key.clone(),
                        entry.descriptor.clone(),
                        image,
                    )))
                });
            if let Some(resource) = &resource {
                entry.initializer = Some(Arc::downgrade(resource));
            }
            entries.push_back(entry);
            if resource.is_some() {
                return resource;
            }
        }
        let pending = entries.iter().any(|entry| {
            entry.key == *key && entry.descriptor.downcast_ref::<ImageLayout>() == Some(&descriptor)
        });
        let lease = if pending || !retain {
            None
        } else {
            self.reserve(&mut entries, bytes)
        };
        if lease.is_none() && !required {
            return None;
        }
        let image = ImageValue::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("evaluation generated value"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        let descriptor: Arc<dyn Any + Send + Sync> = Arc::new(descriptor);
        let publication = {
            let (sender, receiver) = futures_channel::oneshot::channel();
            let result = async move {
                receiver
                    .await
                    .unwrap_or_else(|_| Err(Arc::new(Error::Cancelled)))
            }
            .boxed()
            .shared();
            let token = Arc::new(());
            entries.push_back(Entry {
                token: token.clone(),
                key: key.clone(),
                descriptor: descriptor.clone(),
                value: EntryState::Recording {
                    result: result.downgrade().unwrap(),
                },
                budget: None,
                retain,
                retained: None,
                lease,
                initializer: None,
            });
            Publication {
                sender: Some(sender),
                scope: Arc::downgrade(&self.0),
                token,
                result,
            }
        };
        let token = publication.token.clone();
        let resource = Arc::new(cache::RetainedBuffer::pending(
            key.clone(),
            descriptor,
            image,
            Some(publication),
        ));
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| Arc::ptr_eq(&entry.token, &token))
        {
            entry.initializer = Some(Arc::downgrade(&resource));
        }
        Some(resource)
    }
}

pub(super) struct Publication {
    sender: Option<futures_channel::oneshot::Sender<ImageResult>>,
    scope: Weak<State>,
    token: Arc<()>,
    result: SharedResult,
}
impl Publication {
    pub fn submitted(mut self, image: &Arc<ImageValue>) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Ok(image.clone()));
        }
        let _ = self.result.clone().now_or_never();
        if let Some(scope) = self.scope.upgrade() {
            let scope = Materializations(scope);
            let mut entries = scope.0.entries.lock().unwrap();
            if let Some(index) = entries
                .iter()
                .position(|entry| Arc::ptr_eq(&entry.token, &self.token))
            {
                let entry = entries.remove(index).unwrap();
                scope.completed(
                    &mut entries,
                    entry,
                    image,
                    image.bytes().expect("admitted generated storage"),
                );
            }
        }
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        if self.sender.is_some()
            && let Some(scope) = self.scope.upgrade()
        {
            scope
                .entries
                .lock()
                .unwrap()
                .retain(|entry| !Arc::ptr_eq(&entry.token, &self.token));
        }
    }
}
