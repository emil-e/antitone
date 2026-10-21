//! Document-scoped camera data reuse; native and proxy ownership share one byte LRU.
use crate::{
    Error,
    project::{Development, SourceDevelopment},
    scan::{CameraProxy, DevelopedSource},
    source_revision::FileKey,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

/// Capture before development; grade/crop/correction/target/priority and SourceId are excluded.
#[derive(Clone, Debug)]
pub struct SourceKey {
    // Recheck the original locator, not just its canonical file, after work.
    locator: PathBuf,
    file: FileKey,
    source: SourceDevelopment,
    development: Development,
}

impl SourceKey {
    pub fn capture(
        path: &Path,
        source: &SourceDevelopment,
        development: &Development,
    ) -> Option<Self> {
        let locator = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        Self::from_revision(&locator, FileKey::new(&locator), source, development)
    }

    pub(crate) fn from_revision(
        path: &Path,
        file: FileKey,
        source: &SourceDevelopment,
        development: &Development,
    ) -> Option<Self> {
        file.revision.as_ref()?;
        Some(Self {
            locator: path.to_path_buf(),
            file,
            source: source.clone(),
            development: development.clone(),
        })
    }

    pub(crate) fn current(&self) -> bool {
        self.file.current_at(&self.locator)
    }

    fn matches(&self, source: &DevelopedSource) -> bool {
        let Ok(image) = source.camera_image() else {
            return false;
        };
        let area = source.area();
        let Ok(lens_correction) = source.metadata().lens_identity() else {
            return false;
        };
        self.development == source.metadata().development(source.fixed_wb())
            && self.source
                == SourceDevelopment {
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
        self.file == other.file
            && self.source == other.source
            && self.development == other.development
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
}

#[derive(Clone)]
enum Owned {
    Native(Arc<DevelopedSource>),
    Proxy(Arc<CameraProxy>),
}

enum Borrowed {
    Native(Weak<DevelopedSource>),
    Proxy(Weak<CameraProxy>),
}

impl Owned {
    fn kind(&self) -> Kind {
        match self {
            Self::Native(_) => Kind::Native,
            Self::Proxy(_) => Kind::Proxy,
        }
    }

    fn downgrade(&self) -> Borrowed {
        match self {
            Self::Native(source) => Borrowed::Native(Arc::downgrade(source)),
            Self::Proxy(proxy) => Borrowed::Proxy(Arc::downgrade(proxy)),
        }
    }
}

impl Borrowed {
    fn kind(&self) -> Kind {
        match self {
            Self::Native(_) => Kind::Native,
            Self::Proxy(_) => Kind::Proxy,
        }
    }

    fn live(&self) -> bool {
        match self {
            Self::Native(source) => source.strong_count() != 0,
            Self::Proxy(proxy) => proxy.strong_count() != 0,
        }
    }

    fn upgrade(&self) -> Option<Owned> {
        match self {
            Self::Native(source) => source.upgrade().map(Owned::Native),
            Self::Proxy(proxy) => proxy.upgrade().map(Owned::Proxy),
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(a), Self::Native(b)) => a.ptr_eq(b),
            (Self::Proxy(a), Self::Proxy(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

struct Entry {
    key: SourceKey,
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
        entry.resource.kind() == Kind::Proxy && self.protected.contains(&entry.key)
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

    fn evict(&mut self, key: &SourceKey, removed: &mut Vec<Owned>) {
        for index in 0..self.entries.len() {
            if self.entries[index].key == *key {
                self.remove_ownership(index, removed);
            }
        }
    }
}

/// One store per document; candidates and accepted documents have distinct lifecycles.
/// Active leases survive eviction/closure. Only selected/visible CPU proxies are protected;
/// retaining a full native source is always optional, even while a consumer leases it.
pub struct SourceStore {
    state: Mutex<State>,
    #[cfg(feature = "editor")]
    closed: event_listener::Event,
}

impl SourceStore {
    pub fn new(budget: u64) -> Self {
        Self {
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

    fn get_resource(&self, key: &SourceKey, kind: Kind) -> Option<Owned> {
        if !key.current() {
            self.evict(key);
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
                .position(|entry| entry.key == *key && entry.resource.kind() == kind)?;
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
            self.evict(key);
            None
        }
    }

    pub fn get(&self, key: &SourceKey) -> Option<Arc<DevelopedSource>> {
        match self.get_resource(key, Kind::Native)? {
            Owned::Native(source) => Some(source),
            Owned::Proxy(_) => unreachable!(),
        }
    }

    pub fn get_proxy(&self, key: &SourceKey) -> Option<Arc<CameraProxy>> {
        match self.get_resource(key, Kind::Proxy)? {
            Owned::Proxy(proxy) => Some(proxy),
            Owned::Native(_) => unreachable!(),
        }
    }

    /// Unavailable allocation accounting disables admission, not authoritative processing.
    pub fn publish_source(
        &self,
        key: SourceKey,
        source: Arc<DevelopedSource>,
    ) -> Result<Option<Arc<DevelopedSource>>, Error> {
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
        match self.publish_resource(key, Owned::Native(source), bytes)? {
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
        let bytes = proxy.retained_bytes();
        match self.publish_resource(key, Owned::Proxy(proxy), bytes)? {
            Some(Owned::Proxy(proxy)) => Ok(Some(proxy)),
            None => Ok(None),
            _ => unreachable!(),
        }
    }

    fn publish_resource(
        &self,
        key: SourceKey,
        incoming: Owned,
        bytes: u64,
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
            self.evict(&key);
            Ok(None)
        }
    }

    fn insert(
        state: &mut State,
        key: SourceKey,
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
        target.protect_proxies(
            protected
                .into_iter()
                .filter(|key| allowed.contains(key))
                .collect(),
        );
        for (key, resource, bytes) in moved {
            if allowed.contains(&key)
                && let Err(error) = target.publish_resource(key, resource, bytes)
            {
                eprintln!("CPU cache transfer rejected: {error}");
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
