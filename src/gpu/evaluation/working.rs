//! Run-owned host values remain discoverable independently of optional retention.
use super::*;
use std::{collections::VecDeque, sync::Mutex};

// A bounded live working set, not a bound on an algorithm's required host arrays.
const HOST_REUSE_BYTES: u64 = 64 << 20;

#[derive(Clone)]
pub struct HostWorkingSet(Arc<Mutex<Owned>>);
struct Owned {
    images: VecDeque<Arc<ImageValue>>,
    bytes: u64,
    limit: u64,
}
impl Default for HostWorkingSet {
    fn default() -> Self {
        Self::new(HOST_REUSE_BYTES)
    }
}
impl HostWorkingSet {
    pub(super) fn new(limit: u64) -> Self {
        Self(Arc::new(Mutex::new(Owned {
            images: VecDeque::new(),
            bytes: 0,
            limit,
        })))
    }
    pub(super) fn hold(&self, image: Arc<ImageValue>) {
        let Some(pixels) = image.as_host() else {
            return;
        };
        let bytes = pixels.len() as u64 * 16;
        let mut owned = self.0.lock().unwrap();
        if bytes > owned.limit {
            return;
        }
        if let Some(index) = owned
            .images
            .iter()
            .position(|value| Arc::ptr_eq(value, &image))
        {
            let image = owned.images.remove(index).unwrap();
            owned.images.push_back(image);
            return;
        }
        while owned.bytes + bytes > owned.limit {
            let oldest = owned.images.pop_front().unwrap();
            owned.bytes -= oldest.as_host().unwrap().len() as u64 * 16;
        }
        owned.images.push_back(image);
        owned.bytes += bytes;
    }
}
