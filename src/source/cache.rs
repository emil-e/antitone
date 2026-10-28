//! Document-scoped camera data reuse; native and proxy ownership share one byte LRU.
use super::{FileKey, SourceKey};
use crate::{
    Error,
    embedded_preview::Preview,
    scan::{CameraPixels, DevelopedSource},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};

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
    Proxy(Arc<CameraPixels>),
    Preview(Arc<Preview>),
}

enum Borrowed {
    Native(Weak<DevelopedSource>),
    Proxy(Weak<CameraPixels>),
    Preview(Weak<Preview>),
}

impl Owned {
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
    fn matches(self, key: &ResourceKey) -> bool {
        match (self, key) {
            (Self::Source(a), ResourceKey::Source(b)) => a == b.as_ref(),
            (Self::Preview(a), ResourceKey::Preview(b)) => a == b,
            _ => false,
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

/// Document-scoped optional physical pixel retention. Active leases survive its destruction.
pub struct SourceCache {
    state: Mutex<State>,
}

impl SourceCache {
    pub fn new(budget: u64) -> Self {
        Self {
            state: Mutex::new(State {
                entries: VecDeque::new(),
                protected: Vec::new(),
                budget,
                bytes: 0,
            }),
        }
    }

    fn get_resource(&self, key: KeyRef<'_>, kind: Kind) -> Option<Owned> {
        let mut removed = Vec::new();
        let result = {
            let mut state = self.state.lock().unwrap();
            state.prune();
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
        result
    }

    pub fn get(&self, key: &SourceKey) -> Option<Arc<DevelopedSource>> {
        match self.get_resource(KeyRef::Source(key), Kind::Native)? {
            Owned::Native(source) => Some(source),
            _ => unreachable!(),
        }
    }

    pub fn get_proxy(&self, key: &SourceKey) -> Option<Arc<CameraPixels>> {
        match self.get_resource(KeyRef::Source(key), Kind::Proxy)? {
            Owned::Proxy(proxy) => Some(proxy),
            _ => unreachable!(),
        }
    }

    #[cfg(all(feature = "editor", test))]
    pub(crate) async fn preview(
        &self,
        loader: &super::PreviewLoader,
        file: FileKey,
        read: impl FnOnce() -> super::PreviewResult + Send + 'static,
    ) -> super::PreviewResult {
        if let Some(preview) = self.get_preview(&file) {
            return Ok(Some(preview));
        }
        let result = loader.load(file.clone(), read).await;
        if let Ok(Some(preview)) = &result
            && let Err(error) = self.publish_preview(file, preview.clone())
        {
            log::warn!(error = error.to_string().as_str(); "preview_cache_publication_failed");
        }
        result
    }

    pub(crate) fn get_preview(&self, file: &FileKey) -> Option<Arc<Preview>> {
        match self.get_resource(KeyRef::Preview(file), Kind::Preview)? {
            Owned::Preview(preview) => Some(preview),
            _ => unreachable!(),
        }
    }

    pub(crate) fn publish_preview(
        &self,
        file: FileKey,
        preview: Arc<Preview>,
    ) -> Result<(), Error> {
        let bytes = preview.retained_bytes();
        self.publish_resource(ResourceKey::Preview(file), Owned::Preview(preview), bytes)?;
        Ok(())
    }

    /// Unavailable allocation accounting disables admission, not authoritative processing.
    pub fn publish_source(
        &self,
        key: SourceKey,
        source: Arc<DevelopedSource>,
    ) -> Result<Option<Arc<DevelopedSource>>, Error> {
        if !key.matches(&source) {
            return Err(Error::Project(
                "native source differs from its development key".into(),
            ));
        }

        let Some(bytes) = source.retained_bytes() else {
            return Ok(None);
        };
        self.publish(key, source, bytes)
    }

    /// Explicit nonzero source charges support callers/tests with their own accounting boundary.
    /// Duplicate completion returns the existing lease.
    pub fn publish(
        &self,
        key: SourceKey,
        source: Arc<DevelopedSource>,
        bytes: u64,
    ) -> Result<Option<Arc<DevelopedSource>>, Error> {
        if !key.matches(&source) {
            return Err(Error::Project(
                "native source differs from its development key".into(),
            ));
        }

        match self.publish_resource(
            ResourceKey::Source(Box::new(key)),
            Owned::Native(source),
            bytes,
        )? {
            Some(Owned::Native(source)) => Ok(Some(source)),
            None => Ok(None),
            _ => unreachable!(),
        }
    }

    pub fn publish_proxy(
        &self,
        key: SourceKey,
        proxy: Arc<CameraPixels>,
    ) -> Result<Option<Arc<CameraPixels>>, Error> {
        if !key.matches_proxy(&proxy) {
            return Err(Error::Project(
                "camera proxy differs from its development key".into(),
            ));
        }

        let bytes = proxy.retained_bytes();
        match self.publish_resource(
            ResourceKey::Source(Box::new(key)),
            Owned::Proxy(proxy),
            bytes,
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
        if bytes == 0 {
            return Err(Error::InvalidParameter("source charge must be nonzero"));
        }
        let weak = incoming.downgrade();
        let mut removed = Vec::new();
        let result = {
            let mut state = self.state.lock().unwrap();
            state.prune();
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
        Ok(Some(result))
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
            state.protected = keys;
            state.trim(&mut removed);
        }
        drop(removed);
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

    pub fn can_retain(&self, extra: u64) -> bool {
        let state = self.state.lock().unwrap();
        state
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
}

#[cfg(all(test, feature = "editor"))]
mod tests;

#[cfg(test)]
mod exposure_tests;
#[cfg(test)]
mod preview_tests;
