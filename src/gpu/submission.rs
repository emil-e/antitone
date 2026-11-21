//! One submission lane per device, shared by the host and workers.

#[cfg(any(feature = "ui", test))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(test)]
type Observer = Arc<dyn Fn() + Send + Sync>;

/// Request a device with a queue whose submission and surface configuration share one lock.
pub async fn request_device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Device, Queue), wgpu::RequestDeviceError> {
    let (device, queue) = adapter.request_device(descriptor).await?;
    let lane = Lane {
        queue,
        #[cfg(feature = "ui")]
        device: device.clone(),
        exclusive: Mutex::new(()),
        #[cfg(any(feature = "ui", test))]
        pending: Arc::default(),
    };
    let queue = Queue(
        Arc::new(lane),
        #[cfg(test)]
        Some(Arc::default()),
    );
    Ok((device, queue))
}

/// Clones share submission authority. Encoding, uploads and polling remain outside the lock.
/// Device callbacks must not submit or configure: wgpu can invoke them inside either operation.
#[derive(Clone)]
pub struct Queue(Arc<Lane>, #[cfg(test)] Option<Arc<Observers>>);

struct Lane {
    queue: wgpu::Queue,
    #[cfg(feature = "ui")]
    device: wgpu::Device,
    exclusive: Mutex<()>,
    #[cfg(any(feature = "ui", test))]
    pending: Arc<AtomicUsize>,
}

#[cfg(test)]
#[derive(Default)]
struct Observers {
    encoded: Mutex<Option<Observer>>,
    submitted: Mutex<Option<Observer>>,
}

/// Keeps the host polling through asynchronous readback, including after queue completion.
#[cfg(any(feature = "ui", test))]
pub(crate) struct PendingWork(Arc<AtomicUsize>);

#[cfg(any(feature = "ui", test))]
impl Drop for PendingWork {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl Queue {
    pub fn submit(
        &self,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        #[cfg(test)]
        if let Some(observers) = &self.1 {
            observe(&observers.encoded);
        }
        let index = self.exclusive(|| self.0.queue.submit(commands));
        #[cfg(test)]
        if let Some(observers) = &self.1 {
            observe(&observers.submitted);
        }
        index
    }

    /// Attach completion before another submitter can advance the queue fence.
    pub fn submit_with_completion(
        &self,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
        completion: impl FnOnce() + Send + 'static,
    ) -> wgpu::SubmissionIndex {
        #[cfg(test)]
        if let Some(observers) = &self.1 {
            observe(&observers.encoded);
        }
        let index = self.exclusive(|| {
            #[cfg(any(feature = "ui", test))]
            let pending = self.track();
            let index = self.0.queue.submit(commands);
            self.0.queue.on_submitted_work_done(move || {
                completion();
                #[cfg(any(feature = "ui", test))]
                drop(pending);
            });
            index
        });
        #[cfg(test)]
        if let Some(observers) = &self.1 {
            observe(&observers.submitted);
        }
        index
    }

    pub fn write_buffer(&self, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) {
        self.0.queue.write_buffer(buffer, offset, data);
    }

    pub fn write_texture(
        &self,
        texture: wgpu::TexelCopyTextureInfo<'_>,
        data: &[u8],
        layout: wgpu::TexelCopyBufferLayout,
        size: wgpu::Extent3d,
    ) {
        self.0.queue.write_texture(texture, data, layout, size);
    }

    pub fn get_timestamp_period(&self) -> f32 {
        self.0.queue.get_timestamp_period()
    }

    #[cfg(feature = "ui")]
    pub(crate) fn configure(
        &self,
        surface: &wgpu::Surface<'_>,
        config: &wgpu::SurfaceConfiguration,
    ) {
        self.exclusive(|| surface.configure(&self.0.device, config));
    }

    #[cfg(feature = "ui")]
    pub(crate) fn present(&self, frame: wgpu::SurfaceTexture) {
        self.0.queue.present(frame);
    }

    #[cfg(feature = "ui")]
    pub fn update_egui_texture(
        &self,
        renderer: &mut egui_wgpu::Renderer,
        device: &wgpu::Device,
        id: egui::TextureId,
        delta: &egui::epaint::ImageDelta,
    ) {
        renderer.update_texture(device, &self.0.queue, id, delta);
    }

    // egui's callback ABI receives the raw queue. Antitone callbacks ignore it and
    // use their captured handle for any work they submit independently of the frame.
    #[cfg(feature = "ui")]
    pub fn update_egui_buffers(
        &self,
        renderer: &mut egui_wgpu::Renderer,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        jobs: &[egui::ClippedPrimitive],
        screen: &egui_wgpu::ScreenDescriptor,
    ) -> Vec<wgpu::CommandBuffer> {
        renderer.update_buffers(device, &self.0.queue, encoder, jobs, screen)
    }

    #[cfg(any(feature = "ui", test))]
    pub(crate) fn track(&self) -> PendingWork {
        self.0.pending.fetch_add(1, Ordering::Relaxed);
        PendingWork(self.0.pending.clone())
    }

    #[cfg(any(feature = "ui", test))]
    pub(crate) fn pending(&self) -> bool {
        self.0.pending.load(Ordering::Acquire) != 0
    }

    /// Keeps fixture readbacks off the workload's counters without bypassing its lock.
    #[cfg(test)]
    pub(crate) fn unobserved(&self) -> Self {
        Self(self.0.clone(), None)
    }

    #[cfg(test)]
    pub(crate) fn observe_encoded_resources(&self, observer: Observer) {
        *self.1.as_ref().unwrap().encoded.lock().unwrap() = Some(observer);
    }

    #[cfg(test)]
    pub(crate) fn observe_submissions(&self, observer: Observer) {
        *self.1.as_ref().unwrap().submitted.lock().unwrap() = Some(observer);
    }

    #[cfg(test)]
    pub(crate) fn hold_submission_lock<R>(&self, operation: impl FnOnce() -> R) -> R {
        self.exclusive(operation)
    }

    fn exclusive<R>(&self, operation: impl FnOnce() -> R) -> R {
        let _guard = self.0.exclusive.lock().unwrap();
        operation()
    }
}

#[cfg(test)]
fn observe(slot: &Mutex<Option<Observer>>) {
    let observer = slot.lock().unwrap().clone();
    if let Some(observer) = observer {
        observer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    fn queue() -> (wgpu::Device, Queue) {
        pollster::block_on(async {
            let instance = crate::gpu::instance();
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            request_device(&adapter, &Default::default()).await.unwrap()
        })
    }

    #[test]
    fn pending_readback_keeps_host_polling_until_callback_or_abandonment() {
        let (_, queue) = queue();
        let clone = queue.clone();
        let pending = queue.track();
        assert!(clone.pending());
        let callback = move || drop(pending);
        assert!(clone.pending());
        callback();
        assert!(!clone.pending());
        let pending = queue.track();
        let abandoned = move || drop(pending);
        drop(abandoned);
        assert!(!queue.pending());
    }

    #[test]
    fn fixture_readback_does_not_reenter_the_workload_observer() {
        let (device, queue) = queue();
        let fixture = queue.unobserved();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        queue.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            fixture.submit([device.create_command_encoder(&Default::default()).finish()]);
        }));
        queue.submit([]);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cloned_queue_waits_for_configuration_lock_before_submitting() {
        let (device, queue) = queue();
        for observed in [false, true] {
            let worker_queue = queue.clone();
            let commands = [device.create_command_encoder(&Default::default()).finish()];
            let (ready, started) = mpsc::channel();
            let (finished, submitted) = mpsc::channel();
            let (completed, completion) = mpsc::channel();
            let worker = queue.hold_submission_lock(|| {
                let worker = std::thread::spawn(move || {
                    assert!(worker_queue.0.exclusive.try_lock().is_err());
                    ready.send(()).unwrap();
                    let index = if observed {
                        worker_queue.submit_with_completion(commands, move || {
                            completed.send(()).unwrap();
                        })
                    } else {
                        worker_queue.submit(commands)
                    };
                    finished.send(index).unwrap();
                });
                started.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(matches!(
                    submitted.recv_timeout(Duration::from_millis(100)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ));
                worker
            });
            let index = submitted.recv_timeout(Duration::from_secs(5)).unwrap();
            worker.join().unwrap();
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                })
                .unwrap();
            if observed {
                completion.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            assert!(!queue.pending());
        }
    }
}
