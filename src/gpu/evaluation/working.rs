//! A run owns active input reuse and outstanding GPU work independently of optional retention.
use super::*;
use std::{collections::VecDeque, sync::Mutex};

mod gpu;
#[cfg(all(test, feature = "editor"))]
mod tests;

// A bounded live working set, not a bound on an algorithm's required host arrays.
const HOST_REUSE_BYTES: u64 = 64 << 20;
const SOURCE_REUSE_BYTES: u64 = 64 << 20;

#[derive(Clone)]
pub struct RunWorkingSet {
    host: Arc<Mutex<Owned>>,
    source_tiles: Arc<Mutex<Owned>>,
    gpu: Arc<gpu::GpuRun>,
}
struct Owned {
    images: VecDeque<(Arc<ImageValue>, u64)>,
    bytes: u64,
    limit: u64,
}
impl Owned {
    fn new(limit: u64) -> Self {
        Self {
            images: VecDeque::new(),
            bytes: 0,
            limit,
        }
    }

    fn hold(&mut self, image: Arc<ImageValue>, bytes: u64) {
        if bytes > self.limit {
            return;
        }
        if let Some(index) = self
            .images
            .iter()
            .position(|(value, _)| Arc::ptr_eq(value, &image))
        {
            let held = self.images.remove(index).unwrap();
            self.images.push_back(held);
            return;
        }
        while self.bytes + bytes > self.limit {
            let (_, bytes) = self.images.pop_front().unwrap();
            self.bytes -= bytes;
        }
        self.images.push_back((image, bytes));
        self.bytes += bytes;
    }
}
impl Default for RunWorkingSet {
    fn default() -> Self {
        Self::new(HOST_REUSE_BYTES)
    }
}
impl RunWorkingSet {
    pub(super) fn new(limit: u64) -> Self {
        Self {
            host: Arc::new(Mutex::new(Owned::new(limit))),
            source_tiles: Arc::new(Mutex::new(Owned::new(SOURCE_REUSE_BYTES))),
            gpu: Arc::default(),
        }
    }
    #[cfg(test)]
    pub(super) fn submitted_owns(&self, image: &Arc<ImageValue>) -> bool {
        self.gpu.owns(image)
    }

    pub(super) fn finish_gpu(&self) -> Result<(), Error> {
        self.gpu.finish()
    }

    pub(super) fn admit_gpu(&self, device: &wgpu::Device) -> Result<(), Error> {
        self.gpu.admit(device)
    }

    pub(super) fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        encoder: wgpu::CommandEncoder,
        resources: Allocations,
        submitted: impl FnOnce(),
    ) -> Result<(), Error> {
        self.gpu
            .submit(device, queue, encoder, resources, submitted)
    }

    pub(super) fn hold(&self, image: Arc<ImageValue>) {
        let Some(pixels) = image.as_host() else {
            return;
        };
        let bytes = pixels.len() as u64 * 16;
        self.host.lock().unwrap().hold(image, bytes);
    }

    /// Adjacent units may use a source tile after its first submission completes.
    /// Keep that bounded working set through execution, not through the finished image.
    pub(super) fn hold_source_tile(&self, image: Arc<ImageValue>) {
        let Some(buffer) = image.as_buffer() else {
            return;
        };
        let bytes = buffer.size();
        self.source_tiles.lock().unwrap().hold(image, bytes);
    }
}
