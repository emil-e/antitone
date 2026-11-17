//! A run owns reusable host values and outstanding GPU work independently of optional retention.
use super::*;
use std::{collections::VecDeque, sync::Mutex};

mod gpu;
#[cfg(all(test, feature = "editor"))]
mod tests;

// A bounded live working set, not a bound on an algorithm's required host arrays.
const HOST_REUSE_BYTES: u64 = 64 << 20;

#[derive(Clone)]
pub struct RunWorkingSet {
    host: Arc<Mutex<Owned>>,
    gpu: Arc<gpu::GpuRun>,
}
struct Owned {
    images: VecDeque<Arc<ImageValue>>,
    bytes: u64,
    limit: u64,
}
impl Default for RunWorkingSet {
    fn default() -> Self {
        Self::new(HOST_REUSE_BYTES)
    }
}
impl RunWorkingSet {
    pub(super) fn new(limit: u64) -> Self {
        Self {
            host: Arc::new(Mutex::new(Owned {
                images: VecDeque::new(),
                bytes: 0,
                limit,
            })),
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

    pub(super) fn activate(&self, resources: Allocations) -> Arc<Allocations> {
        self.gpu.activate(resources)
    }

    pub(super) fn activate_buffer(&self, buffer: &wgpu::Buffer) -> Arc<Allocations> {
        let mut resources = Allocations::default();
        resources.buffer(buffer);
        self.activate(resources)
    }

    pub(super) fn admit_gpu(
        &self,
        device: &wgpu::Device,
        bytes: u64,
        allowance: u64,
    ) -> Result<(), Error> {
        self.gpu.admit(device, bytes, allowance)
    }

    pub(super) fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: (&wgpu::Queue, &crate::gpu::submission::SubmissionGate),
        encoder: wgpu::CommandEncoder,
        resources: Allocations,
        admission: (u64, u64),
        submitted: impl FnOnce(),
    ) -> Result<(), Error> {
        self.gpu
            .submit(device, queue, encoder, resources, admission, submitted)
    }

    pub(super) fn hold(&self, image: Arc<ImageValue>) {
        let Some(pixels) = image.as_host() else {
            return;
        };
        let bytes = pixels.len() as u64 * 16;
        let mut owned = self.host.lock().unwrap();
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
