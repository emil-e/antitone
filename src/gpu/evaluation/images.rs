//! Caller-scoped retention of completed immutable images; active consumers independently own pixels.
use super::*;
use std::{any::Any, collections::VecDeque, sync::Mutex};

/// Optional ownership lives with this scope, not the renderer. Descriptors contain numerical
/// provenance and capabilities, never a renderer, publication authority or source-pixel owner.
#[derive(Clone)]
pub struct ImageCache(Arc<State>);
struct State {
    programs: Arc<Programs>,
    retention: cache::Retention,
    capacity: usize,
    entries: Mutex<VecDeque<Entry>>,
}
struct Entry {
    content: RetainedKey,
    descriptor: Arc<dyn Any + Send + Sync>,
    pixels: wgpu::Buffer,
    _lease: Arc<cache::Lease>,
}
pub struct CachedImage<D> {
    pub descriptor: Arc<D>,
    pub pixels: wgpu::Buffer,
}
impl ImageCache {
    pub(super) fn new(
        programs: Arc<Programs>,
        retention: cache::Retention,
        capacity: usize,
    ) -> Self {
        Self(Arc::new(State {
            programs,
            retention,
            capacity,
            entries: Mutex::new(VecDeque::new()),
        }))
    }
    pub(crate) fn belongs_to(&self, context: &Context) -> bool {
        Arc::ptr_eq(&self.0.programs, &context.programs)
    }
    pub fn len(&self) -> usize {
        self.0.entries.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn clear(&self) {
        self.0.entries.lock().unwrap().clear();
    }
    /// Content includes computation, representation and accuracy. The descriptor's capability
    /// predicate checks the requested grid, valid coverage and neighbourhood support.
    pub fn find<D: Any + Send + Sync>(
        &self,
        content: &RetainedKey,
        can_supply: impl Fn(&D) -> bool,
    ) -> Option<CachedImage<D>> {
        let mut entries = self.0.entries.lock().unwrap();
        let index = entries.iter().rposition(|entry| {
            entry.content == *content
                && entry
                    .descriptor
                    .downcast_ref::<D>()
                    .is_some_and(&can_supply)
        })?;
        let entry = entries.remove(index).unwrap();
        let image = CachedImage {
            descriptor: entry.descriptor.clone().downcast().ok()?,
            pixels: entry.pixels.clone(),
        };
        entries.push_back(entry);
        Some(image)
    }
    /// Publish only after successful completion, while pixels remain immutable. A budget miss
    /// drops optional ownership, not the caller's required image. No copy or allocation occurs.
    pub fn retain<D: Any + Send + Sync + PartialEq>(
        &self,
        content: RetainedKey,
        descriptor: Arc<D>,
        pixels: &wgpu::Buffer,
    ) -> bool {
        let bytes = pixels.size();
        if self.0.capacity == 0
            || bytes == 0
            || bytes > self.0.retention.limit()
            || !pixels.usage().contains(wgpu::BufferUsages::STORAGE)
        {
            return false;
        }
        let mut entries = self.0.entries.lock().unwrap();
        if let Some(index) = entries.iter().position(|entry| {
            entry.content == content
                && entry.descriptor.downcast_ref::<D>() == Some(descriptor.as_ref())
        }) {
            let entry = entries.remove(index).unwrap();
            entries.push_back(entry);
            return true;
        }
        if entries.len() == self.0.capacity {
            entries.pop_front();
        }
        let shared = entries
            .iter()
            .find(|entry| entry.pixels == *pixels)
            .map(|entry| entry._lease.clone());
        let lease = if let Some(lease) = shared {
            lease
        } else {
            loop {
                if let Some(lease) = self.0.retention.acquire(bytes) {
                    break Arc::new(lease);
                }
                if entries.pop_front().is_none() {
                    return false;
                }
            }
        };
        entries.push_back(Entry {
            content,
            descriptor,
            pixels: pixels.clone(),
            _lease: lease,
        });
        true
    }
}
