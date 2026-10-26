//! Document-scoped camera data reuse; native and proxy ownership share one byte LRU.
use crate::{
    Error,
    embedded_preview::Preview,
    project::SourceDevelopment,
    scan::{CameraProxy, DevelopedSource},
    source_revision::FileKey,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

/// Capture before development; grade/crop/correction/target/priority and SourceId are excluded.
#[derive(Clone, Debug)]
pub struct SourceKey {
    // Recheck the original locator, not just its canonical file, after work.
    locator: PathBuf,
    file: FileKey,
    source: SourceDevelopment,
}

impl SourceKey {
    /// Capture a load identity even when file stats are unavailable. Missing or stale
    /// stats disable reuse, while the loader still reports authoritative source errors.
    pub fn new(path: &Path, source: &SourceDevelopment) -> Self {
        let locator = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        };
        Self::for_load(&locator, FileKey::new(&locator), source)
    }

    pub(crate) fn for_load(path: &Path, file: FileKey, source: &SourceDevelopment) -> Self {
        Self {
            locator: path.to_path_buf(),
            file,
            source: source.clone(),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.locator
    }

    pub(crate) fn file(&self) -> &FileKey {
        &self.file
    }

    pub(crate) fn development(&self) -> &SourceDevelopment {
        &self.source
    }

    pub fn capture(path: &Path, source: &SourceDevelopment) -> Option<Self> {
        let locator = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        Self::from_revision(&locator, FileKey::new(&locator), source)
    }

    pub(crate) fn from_revision(
        path: &Path,
        file: FileKey,
        source: &SourceDevelopment,
    ) -> Option<Self> {
        file.revision.as_ref()?;
        Some(Self {
            locator: path.to_path_buf(),
            file,
            source: source.clone(),
        })
    }

    pub(crate) fn current(&self) -> bool {
        self.file.current_at(&self.locator)
    }

    pub(crate) fn matches(&self, source: &DevelopedSource) -> bool {
        let Ok(image) = source.camera_image() else {
            return false;
        };
        let area = source.area();
        let Ok(lens_correction) = source.metadata().lens_identity() else {
            return false;
        };
        self.source
            == SourceDevelopment {
                decode: source.metadata().development(source.fixed_wb()),
                lens_correction,
                dng_development: source.metadata().dng_identity(),
                dimensions: [image.width, image.height],
                active_area: [area.x, area.y, area.width, area.height],
                camera_model: source.camera_model().to_owned(),
                camera_to_ap1: source.camera_to_ap1().to_cols_array_2d(),
                input_floor: source.input_floor(),
            }
    }

    fn matches_proxy(&self, proxy: &CameraProxy) -> bool {
        proxy.width > 0
            && proxy.height > 0
            && proxy.width <= CameraProxy::MAX_EDGE
            && proxy.height <= CameraProxy::MAX_EDGE
            && proxy.pixels.len() == proxy.width as usize * proxy.height as usize
            && proxy.matrix.to_cols_array_2d() == self.source.camera_to_ap1
    }
}

impl PartialEq for SourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.source == other.source
    }
}

/// Optional retained ownership and evicted-but-leased storage, not process footprint.
/// Protected proxy ownership is reported separately by `protected_bytes`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Charges {
    pub retained: u64,
    pub leased_only: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Native,
    Proxy,
    Preview,
}

#[derive(Clone)]
enum Owned {
    Native(Arc<DevelopedSource>),
    Proxy(Arc<CameraProxy>),
    Preview(Arc<Preview>),
}

enum Borrowed {
    Native(Weak<DevelopedSource>),
    Proxy(Weak<CameraProxy>),
    Preview(Weak<Preview>),
}

impl Owned {
    fn reference(&self) -> Option<&crate::exposure_reference::ReferenceExposure> {
        match self {
            Self::Native(source) => source.reference_exposure.get(),
            Self::Proxy(proxy) => proxy.reference_exposure.get(),
            Self::Preview(_) => None,
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Self::Native(_) => Kind::Native,
            Self::Proxy(_) => Kind::Proxy,
            Self::Preview(_) => Kind::Preview,
        }
    }

    fn downgrade(&self) -> Borrowed {
        match self {
            Self::Native(source) => Borrowed::Native(Arc::downgrade(source)),
            Self::Proxy(proxy) => Borrowed::Proxy(Arc::downgrade(proxy)),
            Self::Preview(preview) => Borrowed::Preview(Arc::downgrade(preview)),
        }
    }
}

impl Borrowed {
    fn kind(&self) -> Kind {
        match self {
            Self::Native(_) => Kind::Native,
            Self::Proxy(_) => Kind::Proxy,
            Self::Preview(_) => Kind::Preview,
        }
    }

    fn live(&self) -> bool {
        match self {
            Self::Native(source) => source.strong_count() != 0,
            Self::Proxy(proxy) => proxy.strong_count() != 0,
            Self::Preview(preview) => preview.strong_count() != 0,
        }
    }

    fn upgrade(&self) -> Option<Owned> {
        match self {
            Self::Native(source) => source.upgrade().map(Owned::Native),
            Self::Proxy(proxy) => proxy.upgrade().map(Owned::Proxy),
            Self::Preview(preview) => preview.upgrade().map(Owned::Preview),
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(a), Self::Native(b)) => a.ptr_eq(b),
            (Self::Proxy(a), Self::Proxy(b)) => a.ptr_eq(b),
            (Self::Preview(a), Self::Preview(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

#[derive(Clone, PartialEq)]
enum ResourceKey {
    Source(Box<SourceKey>),
    Preview(FileKey),
}

#[derive(Clone, Copy)]
enum KeyRef<'a> {
    Source(&'a SourceKey),
    Preview(&'a FileKey),
}

impl KeyRef<'_> {
    fn current(self) -> bool {
        match self {
            Self::Source(key) => key.current(),
            Self::Preview(file) => file.current_at(&file.path),
        }
    }

    fn matches(self, key: &ResourceKey) -> bool {
        match (self, key) {
            (Self::Source(a), ResourceKey::Source(b)) => a == b.as_ref(),
            (Self::Preview(a), ResourceKey::Preview(b)) => a == b,
            _ => false,
        }
    }
}

impl ResourceKey {
    fn as_ref(&self) -> KeyRef<'_> {
        match self {
            Self::Source(key) => KeyRef::Source(key),
            Self::Preview(file) => KeyRef::Preview(file),
        }
    }

    fn current(&self) -> bool {
        self.as_ref().current()
    }

    #[cfg(feature = "editor")]
    fn allowed(&self, sources: &[SourceKey]) -> bool {
        match self {
            Self::Source(key) => sources.contains(key.as_ref()),
            Self::Preview(file) => sources.iter().any(|source| source.file == *file),
        }
    }
}

struct Entry {
    key: ResourceKey,
    resource: Borrowed,
    retained: Option<Owned>,
    bytes: u64,
}

struct State {
    entries: VecDeque<Entry>,
    protected: Vec<SourceKey>,
    budget: u64,
    bytes: u64,
    closed: bool,
}

impl State {
    fn prune(&mut self) {
        self.entries.retain(|entry| entry.resource.live());
    }

    fn essential(&self, entry: &Entry) -> bool {
        entry.resource.kind() == Kind::Proxy
            && matches!(&entry.key, ResourceKey::Source(key) if self.protected.contains(key.as_ref()))
    }

    fn protected_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.retained.is_some() && self.essential(entry))
            .map(|entry| entry.bytes)
            .sum()
    }

    fn charges(&self) -> Charges {
        Charges {
            retained: self.bytes - self.protected_bytes(),
            leased_only: self
                .entries
                .iter()
                .filter(|entry| entry.retained.is_none() && entry.resource.live())
                .map(|entry| entry.bytes)
                .sum(),
        }
    }

    fn remove_ownership(&mut self, index: usize, removed: &mut Vec<Owned>) {
        let entry = &mut self.entries[index];
        if let Some(resource) = entry.retained.take() {
            self.bytes -= entry.bytes;
            removed.push(resource);
        }
    }

    fn trim(&mut self, removed: &mut Vec<Owned>) {
        while self.charges().retained > self.budget {
            let index = self
                .entries
                .iter()
                .position(|entry| entry.retained.is_some() && !self.essential(entry))
                .unwrap();
            self.remove_ownership(index, removed);
        }
    }

    fn keep(&mut self, entry: &mut Entry, resource: Owned) {
        if entry.retained.is_none() && (entry.bytes <= self.budget || self.essential(entry)) {
            self.bytes += entry.bytes;
            entry.retained = Some(resource);
        }
    }

    fn evict(&mut self, key: KeyRef<'_>, removed: &mut Vec<Owned>) {
        for index in 0..self.entries.len() {
            if key.matches(&self.entries[index].key) {
                self.remove_ownership(index, removed);
            }
        }
    }
}

#[cfg(test)]
mod exposure_tests;
#[cfg(test)]
mod preview_tests;

type PreviewResult = Result<Option<Arc<Preview>>, String>;
type PreviewFlight = OnceLock<PreviewResult>;
type PreviewLease = (FileKey, Weak<PreviewFlight>, Weak<Preview>);
type ExposureMemo = Arc<OnceLock<crate::exposure_reference::ReferenceExposure>>;

pub struct PreparedSource {
    pub source: Arc<DevelopedSource>,
    pub proxy: Option<Arc<CameraProxy>>,
}

/// Pixel ownership is scoped to a document generation; same-document forks share
/// immutable metering and in-flight preview work. Active leases survive eviction/closure.
pub struct SourceStore {
    state: Mutex<State>,
    preview_flights: Arc<Mutex<Vec<PreviewLease>>>,
    exposures: Arc<Mutex<Vec<(SourceKey, ExposureMemo)>>>,
    #[cfg(feature = "editor")]
    closed: event_listener::Event,
}

impl SourceStore {
    pub fn new(budget: u64) -> Self {
        Self {
            preview_flights: Arc::new(Mutex::new(Vec::new())),
            exposures: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "editor")]
            closed: event_listener::Event::new(),
            state: Mutex::new(State {
                entries: VecDeque::new(),
                protected: Vec::new(),
                budget,
                bytes: 0,
                closed: false,
            }),
        }
    }

    /// Same-document candidates isolate pixel publication but retain completed metering
    /// and in-flight reference work even after optional pixel ownership is evicted.
    pub fn fork(&self, budget: u64) -> Self {
        let mut fork = Self::new(budget);
        fork.exposures = self.exposures.clone();
        fork.preview_flights = self.preview_flights.clone();
        fork
    }

    // Preparation interprets an already captured pixel lease. Acquisition/publication owns
    // locator freshness; looking up that snapshot must not stat or relabel it.
    fn peek_resource(&self, key: KeyRef<'_>, kind: Kind) -> Option<Owned> {
        self.state
            .lock()
            .unwrap()
            .entries
            .iter()
            .find(|entry| key.matches(&entry.key) && entry.resource.kind() == kind)
            .and_then(|entry| entry.resource.upgrade())
    }

    fn get_resource(&self, key: KeyRef<'_>, kind: Kind) -> Option<Owned> {
        if !key.current() {
            self.evict_resource(key);
            return None;
        }
        let mut removed = Vec::new();
        let result = {
            let mut state = self.state.lock().unwrap();
            state.prune();
            if state.closed {
                return None;
            }
            let index = state
                .entries
                .iter()
                .position(|entry| key.matches(&entry.key) && entry.resource.kind() == kind)?;
            let mut entry = state.entries.remove(index).unwrap();
            let result = entry.resource.upgrade();
            if let Some(resource) = &result {
                state.keep(&mut entry, resource.clone());
            }
            state.entries.push_back(entry);
            state.trim(&mut removed);
            result
        };
        drop(removed);
        if key.current() {
            result
        } else {
            self.evict_resource(key);
            None
        }
    }

    pub fn get(&self, key: &SourceKey) -> Option<Arc<DevelopedSource>> {
        match self.get_resource(KeyRef::Source(key), Kind::Native)? {
            Owned::Native(source) => Some(source),
            _ => unreachable!(),
        }
    }

    pub fn get_proxy(&self, key: &SourceKey) -> Option<Arc<CameraProxy>> {
        match self.get_resource(KeyRef::Source(key), Kind::Proxy)? {
            Owned::Proxy(proxy) => Some(proxy),
            _ => unreachable!(),
        }
    }

    /// Snapshot completed values only; saving must not start metering unvisited photos.
    pub fn exposure_metadata(
        &self,
        project: &crate::project::Project,
        path: impl Fn(crate::project::SourceId) -> Result<PathBuf, Error>,
    ) -> crate::exposure_metadata::Metadata {
        let ready = self
            .exposures
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.get()?.clone())))
            .collect::<Vec<_>>();
        let mut metadata = crate::exposure_metadata::Metadata::default();
        if project.input.is_negative() {
            return metadata;
        }
        for source in project
            .sources
            .iter()
            .filter(|source| project.order.contains(&source.id))
        {
            let Some(key) = path(source.id)
                .ok()
                .and_then(|path| SourceKey::capture(&path, &source.development))
            else {
                continue;
            };
            let Some((_, value)) = ready.iter().find(|(existing, _)| *existing == key) else {
                continue;
            };
            metadata.entries.push(crate::exposure_metadata::Entry {
                source: source.id,
                development: source.development.clone(),
                revision: key.file.revision.clone().unwrap(),
                reference: value.clone(),
            });
        }
        metadata
    }

    pub fn restore_exposure_metadata(
        &self,
        metadata: &crate::exposure_metadata::Metadata,
        project: &crate::project::Project,
        path: impl Fn(crate::project::SourceId) -> Result<PathBuf, Error>,
    ) {
        for entry in metadata.for_project(project).entries {
            let Some(key) = path(entry.source)
                .ok()
                .and_then(|path| SourceKey::capture(&path, &entry.development))
            else {
                continue;
            };
            if key.file.revision.as_ref() == Some(&entry.revision) {
                let _ = self.exposure_memo(&key).set(entry.reference);
            }
        }
    }

    /// Resolve a developed photo's starting interpretation on a worker. Pixel eviction
    /// cannot re-meter the same capture; an optional returned proxy reuses metering work.
    pub fn prepare_source(
        &self,
        input: crate::processing::InputSettings,
        source: &Arc<DevelopedSource>,
    ) -> Result<PreparedSource, Error> {
        let development = SourceDevelopment::from_metadata(&source.metadata(), source.fixed_wb())?;
        let key = SourceKey::from_revision(&source.file.path, source.file.clone(), &development);
        let source = key
            .as_ref()
            .and_then(
                |key| match self.peek_resource(KeyRef::Source(key), Kind::Native) {
                    Some(Owned::Native(source)) => Some(source),
                    _ => None,
                },
            )
            .unwrap_or_else(|| source.clone());
        let mut prepared = crate::photo_preparation::native(
            self,
            &source.file.path,
            &source.file,
            &development,
            input,
            &source,
        )?;
        if let Some(key) = key {
            if !Arc::ptr_eq(&source, &prepared.source)
                && let Some(bytes) = prepared.source.retained_bytes()
                && let Some(Owned::Native(source)) = self.publish_interpretation(
                    ResourceKey::Source(Box::new(key.clone())),
                    Owned::Native(prepared.source.clone()),
                    bytes,
                    true,
                )?
            {
                prepared.source = source;
            }
            if let Some(proxy) = &prepared.proxy {
                prepared.proxy = Some(self.prepare_proxy(&key, input, proxy)?);
            }
        }
        Ok(prepared)
    }

    /// Resolve cached pixels against the document's canonical outcome; a restored value
    /// takes precedence over disposable hints without copying the shared pixel buffer.
    pub fn prepare_proxy(
        &self,
        key: &SourceKey,
        input: crate::processing::InputSettings,
        proxy: &Arc<CameraProxy>,
    ) -> Result<Arc<CameraProxy>, Error> {
        if !key.matches_proxy(proxy) {
            return Err(Error::Project(
                "camera proxy differs from its preparation key".into(),
            ));
        }
        let cached = match self.peek_resource(KeyRef::Source(key), Kind::Proxy) {
            Some(Owned::Proxy(proxy)) => Some(proxy),
            _ => None,
        };
        let data = cached.clone().unwrap_or_else(|| proxy.clone());
        let resolved = crate::photo_preparation::proxy(
            self,
            &key.locator,
            &key.file,
            &key.source,
            input,
            &data,
        )?;
        if cached
            .as_ref()
            .is_some_and(|cached| Arc::ptr_eq(cached, &resolved))
        {
            return Ok(resolved);
        }
        match self.publish_interpretation(
            ResourceKey::Source(Box::new(key.clone())),
            Owned::Proxy(resolved.clone()),
            resolved.retained_bytes(),
            true,
        )? {
            Some(Owned::Proxy(proxy)) => Ok(proxy),
            _ => Ok(resolved),
        }
    }

    pub(crate) fn exposure_memo(
        &self,
        key: &SourceKey,
    ) -> Arc<OnceLock<crate::exposure_reference::ReferenceExposure>> {
        let mut exposures = self.exposures.lock().unwrap();
        if let Some((_, memo)) = exposures.iter().find(|(existing, _)| existing == key) {
            return memo.clone();
        }
        let memo = Arc::new(OnceLock::new());
        exposures.push((key.clone(), memo.clone()));
        memo
    }

    pub(crate) fn completed_reference(
        &self,
        key: &SourceKey,
    ) -> Option<crate::exposure_reference::ReferenceExposure> {
        self.exposures
            .lock()
            .unwrap()
            .iter()
            .find(|(existing, _)| existing == key)
            .and_then(|(_, memo)| memo.get())
            .cloned()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn positive_matrix(
        &self,
        key: &SourceKey,
        input: &crate::processing::InputSettings,
    ) -> Result<glam::DMat3, Error> {
        crate::scan::positive_matrix(
            input,
            glam::DMat3::from_cols_array_2d(&key.source.camera_to_ap1),
            self.completed_reference(key).as_ref(),
        )
    }

    /// Worker-only preview acquisition: extraction/decoding is single-flight, and decoded
    /// data competes with native/proxy ownership in this store's byte budget.
    pub(crate) fn preview(
        &self,
        file: &FileKey,
        load: impl FnOnce() -> PreviewResult,
    ) -> PreviewResult {
        let key = ResourceKey::Preview(file.clone());
        if !key.current() {
            return Err("embedded preview source changed".into());
        }
        if let Some(Owned::Preview(preview)) = self.get_resource(key.as_ref(), Kind::Preview) {
            return Ok(Some(preview));
        }
        let flight = {
            let mut flights = self.preview_flights.lock().unwrap();
            flights.retain(|(_, flight, ready)| {
                flight.strong_count() != 0 || ready.strong_count() != 0
            });
            match flights
                .iter()
                .find(|(key, _, _)| key == file)
                .and_then(|(_, weak, ready)| {
                    weak.upgrade().or_else(|| {
                        ready
                            .upgrade()
                            .map(|preview| Arc::new(OnceLock::from(Ok(Some(preview)))))
                    })
                }) {
                Some(flight) => flight,
                None => {
                    let flight = Arc::new(OnceLock::new());
                    if let Some((_, pending, _)) =
                        flights.iter_mut().find(|(key, _, _)| key == file)
                    {
                        *pending = Arc::downgrade(&flight);
                    } else {
                        flights.push((file.clone(), Arc::downgrade(&flight), Weak::new()));
                    }
                    flight
                }
            }
        };
        let result = flight
            .get_or_init(|| {
                // Another producer may have completed between the first lookup and flight admission.
                if let Some(Owned::Preview(preview)) =
                    self.get_resource(key.as_ref(), Kind::Preview)
                {
                    Ok(Some(preview))
                } else {
                    load()
                }
            })
            .clone()?;
        if !key.current() {
            return Err("embedded preview source changed during extraction".into());
        }
        if let Some(preview) = &result {
            if let Some((_, _, ready)) = self
                .preview_flights
                .lock()
                .unwrap()
                .iter_mut()
                .find(|(key, _, _)| key == file)
            {
                *ready = Arc::downgrade(preview);
            }
            self.publish_resource(
                key,
                Owned::Preview(preview.clone()),
                preview.retained_bytes(),
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(result)
    }

    /// Unavailable allocation accounting disables admission, not authoritative processing.
    pub fn publish_source(
        &self,
        key: SourceKey,
        source: Arc<DevelopedSource>,
    ) -> Result<Option<Arc<DevelopedSource>>, Error> {
        if !key.current() {
            return Ok(None);
        }
        if !key.matches(&source) {
            return Err(Error::Project(
                "native source differs from its development key".into(),
            ));
        }
        let source = self
            .completed_reference(&key)
            .map_or_else(|| source.clone(), |value| source.interpreted(&value));
        let Some(bytes) = source.retained_bytes() else {
            return Ok(None);
        };
        self.publish(key, source, bytes)
    }

    /// Explicit nonzero source charges support callers/tests with their own accounting boundary.
    /// Duplicate completion returns the existing lease; None means stale/closed.
    pub fn publish(
        &self,
        key: SourceKey,
        source: Arc<DevelopedSource>,
        bytes: u64,
    ) -> Result<Option<Arc<DevelopedSource>>, Error> {
        if !key.current() {
            return Ok(None);
        }
        if !key.matches(&source) {
            return Err(Error::Project(
                "native source differs from its development key".into(),
            ));
        }
        let replace = source.reference_exposure.get().is_some()
            && self.completed_reference(&key).as_ref() == source.reference_exposure.get();
        match self.publish_interpretation(
            ResourceKey::Source(Box::new(key)),
            Owned::Native(source),
            bytes,
            replace,
        )? {
            Some(Owned::Native(source)) => Ok(Some(source)),
            None => Ok(None),
            _ => unreachable!(),
        }
    }

    pub fn publish_proxy(
        &self,
        key: SourceKey,
        proxy: Arc<CameraProxy>,
    ) -> Result<Option<Arc<CameraProxy>>, Error> {
        if !key.current() {
            return Ok(None);
        }
        if !key.matches_proxy(&proxy) {
            return Err(Error::Project(
                "camera proxy differs from its development key".into(),
            ));
        }
        // CPU-only warming does not meter, but must carry any already-known canonical
        // outcome before a pending selected preview can consume these pixels.
        let reference = self.completed_reference(&key);
        let proxy = reference
            .as_ref()
            .map_or_else(|| proxy.clone(), |value| proxy.interpreted(value));
        let bytes = proxy.retained_bytes();
        match self.publish_interpretation(
            ResourceKey::Source(Box::new(key)),
            Owned::Proxy(proxy),
            bytes,
            reference.is_some(),
        )? {
            Some(Owned::Proxy(proxy)) => Ok(Some(proxy)),
            None => Ok(None),
            _ => unreachable!(),
        }
    }

    fn publish_resource(
        &self,
        key: ResourceKey,
        incoming: Owned,
        bytes: u64,
    ) -> Result<Option<Owned>, Error> {
        self.publish_interpretation(key, incoming, bytes, false)
    }

    fn publish_interpretation(
        &self,
        key: ResourceKey,
        incoming: Owned,
        bytes: u64,
        replace: bool,
    ) -> Result<Option<Owned>, Error> {
        if bytes == 0 {
            return Err(Error::InvalidParameter("source charge must be nonzero"));
        }
        if !key.current() {
            return Ok(None);
        }
        let weak = incoming.downgrade();
        let mut removed = Vec::new();
        let result = {
            let mut state = self.state.lock().unwrap();
            state.prune();
            if state.closed {
                return Ok(None);
            }
            let index = state
                .entries
                .iter()
                .position(|entry| entry.key == key && entry.resource.kind() == incoming.kind());
            let result = if let Some(index) = index {
                let entry = &state.entries[index];
                if let Some(existing) = entry.resource.upgrade() {
                    if entry.resource.same(&weak) && entry.bytes != bytes {
                        // Incoming owns the same data beyond guard release, including on error.
                        return Err(Error::InvalidParameter("existing source charge differs"));
                    }
                    let mut entry = state.entries.remove(index).unwrap();
                    let existing = if replace
                        && !entry.resource.same(&weak)
                        && existing.reference() != incoming.reference()
                    {
                        if let Some(owned) = entry.retained.take() {
                            state.bytes -= entry.bytes;
                            removed.push(owned);
                        }
                        removed.push(existing);
                        entry.resource = incoming.downgrade();
                        entry.bytes = bytes;
                        incoming.clone()
                    } else {
                        existing
                    };
                    state.keep(&mut entry, existing.clone());
                    state.entries.push_back(entry);
                    existing
                } else {
                    state.entries.remove(index);
                    Self::insert(&mut state, key.clone(), &incoming, bytes)?;
                    incoming
                }
            } else {
                if state.entries.iter().any(|entry| entry.resource.same(&weak)) {
                    return Err(Error::InvalidParameter(
                        "source already belongs to another key",
                    ));
                }
                Self::insert(&mut state, key.clone(), &incoming, bytes)?;
                incoming
            };
            // Trim atomically with publication; the result keeps demanded data alive.
            state.trim(&mut removed);
            result
        };
        drop(removed);
        if key.current() {
            Ok(Some(result))
        } else {
            self.evict_resource(key.as_ref());
            Ok(None)
        }
    }

    fn insert(
        state: &mut State,
        key: ResourceKey,
        incoming: &Owned,
        bytes: u64,
    ) -> Result<(), Error> {
        let charges = state.charges();
        state
            .bytes
            .checked_add(charges.leased_only)
            .and_then(|total| total.checked_add(bytes))
            .ok_or(Error::NumericalOverflow)?;
        let mut entry = Entry {
            key,
            resource: incoming.downgrade(),
            retained: None,
            bytes,
        };
        state.keep(&mut entry, incoming.clone());
        state.entries.push_back(entry);
        Ok(())
    }

    /// Drop all cache ownership under this stale key; leases remain valid.
    pub fn evict(&self, key: &SourceKey) {
        self.evict_resource(KeyRef::Source(key));
    }

    fn evict_resource(&self, key: KeyRef<'_>) {
        let mut removed = Vec::new();
        self.state.lock().unwrap().evict(key, &mut removed);
        drop(removed);
    }

    pub fn set_budget(&self, budget: u64) {
        let mut removed = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            state.budget = budget;
            state.trim(&mut removed);
        }
        drop(removed);
    }

    /// Protect only proxy ownership. Newly unprotected proxies compete in the same byte LRU.
    pub fn protect_proxies(&self, keys: Vec<SourceKey>) {
        let mut removed = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            if !state.closed {
                state.protected = keys;
            }
            state.trim(&mut removed);
        }
        drop(removed);
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn proxy_count(&self) -> usize {
        let state = self.state.lock().unwrap();
        state
            .entries
            .iter()
            .filter(|entry| entry.resource.kind() == Kind::Proxy && entry.resource.live())
            .count()
    }

    /// Accepted same-document replacement only. Retire old ownership before admission;
    /// transfer handles/metadata, never pixels. Drained handles are retiring resources,
    /// not instant physical reclamation credit. No lock spans validation or destruction.
    #[cfg(feature = "editor")]
    pub(crate) fn transfer_to(&self, target: &Self, allowed: &[SourceKey]) {
        let (moved, protected) = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            let protected = std::mem::take(&mut state.protected);
            state.budget = 0;
            state.bytes = 0;
            let moved = state
                .entries
                .iter_mut()
                .filter_map(|entry| {
                    entry
                        .retained
                        .take()
                        .map(|resource| (entry.key.clone(), resource, entry.bytes))
                })
                .collect::<Vec<_>>();
            (moved, protected)
        };
        let exposures = self
            .exposures
            .lock()
            .unwrap()
            .iter()
            .filter(|(key, _)| allowed.contains(key))
            .filter_map(|(key, memo)| Some((key.clone(), memo.get()?.clone())))
            .collect::<Vec<_>>();
        for (key, value) in exposures {
            let _ = target.exposure_memo(&key).set(value);
        }
        target.protect_proxies(
            protected
                .into_iter()
                .filter(|key| allowed.contains(key))
                .collect(),
        );
        for (key, resource, bytes) in moved {
            if key.allowed(allowed)
                && let Err(error) = target.publish_resource(key, resource, bytes)
            {
                log::warn!(error = error.to_string().as_str(); "cpu_cache_transfer_rejected");
            }
        }
    }

    pub fn can_retain(&self, extra: u64) -> bool {
        let state = self.state.lock().unwrap();
        !state.closed
            && state
                .charges()
                .retained
                .checked_add(extra)
                .is_some_and(|bytes| bytes <= state.budget)
    }

    pub fn charges(&self) -> Charges {
        let mut state = self.state.lock().unwrap();
        state.prune();
        state.charges()
    }

    pub fn protected_bytes(&self) -> u64 {
        self.state.lock().unwrap().protected_bytes()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn listen_for_close(&self) -> event_listener::EventListener {
        self.closed.listen()
    }

    pub fn close(&self) {
        let mut removed = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            state.protected.clear();
            state.budget = 0;
            state.trim(&mut removed);
        }
        #[cfg(feature = "editor")]
        self.closed.notify(usize::MAX);
        drop(removed);
    }
}
